use crate::{
    fluid::{FluidCell, FluidKind},
    lighting::SkyLight,
    model::{resolve_block_variants, ResolvedModel},
    pack::{PackStack, ResourceId},
    scene::{BiomeSample, Block, BlockPos, ChunkPos, Scene},
    texture_mips,
};
use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use image::{imageops::FilterType, RgbaImage};
use minecraftoss_player::items::WorldItems;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub sky_light: f32,
    pub block_light: f32,
}
impl Vertex {
}
/// A terrain section vertex as the GPU stores it: 28 bytes, the size of
/// vanilla's block vertex. Colours are bytes as `ARGB.colorFromFloat`
/// makes them (`floor(value * 255)`), and light keeps vanilla's lightmap
/// coordinate precision (a level in sixteenths: smooth lighting averages
/// four levels, so quarter levels are exact).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SectionVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [u8; 4],
    /// Sky then block light, times 16.
    pub light: [u8; 2],
    pub pad: [u8; 2],
}

impl SectionVertex {

    pub fn from_vertex(v: &Vertex) -> Self {
        // `ARGB.as8BitChannel`.
        let channel = |value: f32| (value * 255.0).floor().clamp(0.0, 255.0) as u8;
        let light = |level: f32| (level * 16.0).round().clamp(0.0, 255.0) as u8;
        Self {
            position: v.position,
            uv: v.uv,
            color: v.color.map(channel),
            light: [light(v.sky_light), light(v.block_light)],
            pad: [0; 2],
        }
    }
}

/// A terrain section's mesh in the GPU vertex format.
#[derive(Default)]
pub struct SectionMesh {
    pub vertices: Vec<SectionVertex>,
    pub indices: Vec<u32>,
    pub transparent_start: Option<u32>,
}

impl From<ChunkMesh> for SectionMesh {
    fn from(mesh: ChunkMesh) -> Self {
        Self {
            vertices: mesh.vertices.iter().map(SectionVertex::from_vertex).collect(),
            indices: mesh.indices,
            transparent_start: mesh.transparent_start,
        }
    }
}

#[derive(Default)]
pub struct ChunkMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub transparent_start: Option<u32>,
    pub faces: usize,
}
pub struct Atlas {
    pub pixels: RgbaImage,
    pub mipmaps: Vec<RgbaImage>,
    slots: HashMap<ResourceId, [f32; 4]>,
    pub missing: Vec<ResourceId>,
    pub animated: Vec<ResourceId>,
    pub animated_tiles: Vec<AnimatedTile>,
}
pub struct AnimatedTile {
    pub origin: (u32, u32),
    pub frames: Vec<Vec<RgbaImage>>,
    pub sequence: Vec<(usize, u32)>,
    pub interpolate: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationFrameState {
    pub current: usize,
    pub next: usize,
    pub progress_millis: u16,
}
impl AnimatedTile {
    pub fn state_at(&self, tick: u64) -> AnimationFrameState {
        let total: u64 = self
            .sequence
            .iter()
            .map(|(_, duration)| *duration as u64)
            .sum();
        let mut remaining = tick % total;
        for (position, &(frame, duration)) in self.sequence.iter().enumerate() {
            if remaining < duration as u64 {
                let next = self.sequence[(position + 1) % self.sequence.len()].0;
                let progress_millis = ((remaining as f32 / duration as f32) * 1000.0_f32) as u16;
                return AnimationFrameState {
                    current: frame,
                    next,
                    progress_millis,
                };
            }
            remaining -= duration as u64;
        }
        unreachable!("valid animation sequence has a positive duration")
    }
    pub fn frame_at(&self, tick: u64) -> usize {
        self.state_at(tick).current
    }
    pub fn image_at(
        &self,
        state: AnimationFrameState,
        level: usize,
    ) -> std::borrow::Cow<'_, RgbaImage> {
        let current = &self.frames[state.current][level];
        if !self.interpolate || state.progress_millis == 0 || state.current == state.next {
            return std::borrow::Cow::Borrowed(current);
        }
        let next = &self.frames[state.next][level];
        let progress = state.progress_millis as f32 / 1000.0;
        let mut image = RgbaImage::new(current.width(), current.height());
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let a = current.get_pixel(x, y);
            let b = next.get_pixel(x, y);
            for channel in 0..4 {
                let first = a[channel] as f32 / 255.0;
                let second = b[channel] as f32 / 255.0;
                pixel[channel] = ((first * (1.0 - progress) + second * progress) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
        std::borrow::Cow::Owned(image)
    }
}
impl Atlas {
    pub fn region(&self, id: &ResourceId) -> [f32; 4] {
        self.slots
            .get(id)
            .copied()
            .or_else(|| {
                self.slots
                    .get(&ResourceId::parse("minecraft:missingno").unwrap())
                    .copied()
            })
            .unwrap_or([0.0, 0.0, 1.0, 1.0])
    }
    /// Entity models address individual pixels inside a skin. The regular
    /// region is inset by half a texel to protect whole block sprites from
    /// neighbouring atlas tiles; scaling that inset over a skin moves inner
    /// UV boundaries into the adjacent body-part texture.
    /// Entity skins need their exact pixel boundaries; see `region_exact`.
    pub fn entity_region(&self, id: &ResourceId) -> [f32; 4] {
        self.region_exact(id)
    }
    pub fn contains(&self, id: &ResourceId) -> bool {
        self.slots.contains_key(id)
    }
    fn region_exact(&self, id: &ResourceId) -> [f32; 4] {
        let [u0, v0, u1, v1] = self.region(id);
        let half_texel = 0.5 / self.pixels.width() as f32;
        [
            u0 - half_texel,
            v0 - half_texel,
            u1 + half_texel,
            v1 + half_texel,
        ]
    }
}
pub struct Build {
    pub atlas: Arc<Atlas>,
    pub chunks: BTreeMap<ChunkPos, ChunkMesh>,
    pub sky_light: Option<Arc<SkyLight>>,
    pub scene_revision: u64,
    pub pack_generation: u64,
}
pub struct BiomeTint {
    grass: Option<RgbaImage>,
    foliage: Option<RgbaImage>,
}
impl BiomeTint {
    pub fn from_pack(packs: &PackStack) -> Result<Self> {
        let mut tint = Self {
            grass: None,
            foliage: None,
        };
        for (name, output) in [("grass", &mut tint.grass), ("foliage", &mut tint.foliage)] {
            let id = ResourceId::parse(&format!("minecraft:colormap/{name}"))?;
            if let Some(bytes) = packs.texture(&id)? {
                let map = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?
                    .to_rgba8();
                if map.width() >= 256 && map.height() >= 256 {
                    *output = Some(map);
                }
            }
        }
        Ok(tint)
    }
    fn color(
        map: &Option<RgbaImage>,
        override_color: Option<[u8; 3]>,
        biome: BiomeSample,
        fallback: [u8; 3],
    ) -> [f32; 3] {
        let rgb = override_color
            .or_else(|| {
                map.as_ref().map(|map| {
                    // 26.3 ColorMapColorUtil: rainfall is multiplied by temperature.
                    let temp = biome.temperature.clamp(0.0, 1.0);
                    let rain = biome.downfall.clamp(0.0, 1.0) * temp;
                    let pixel =
                        map.get_pixel(((1.0 - temp) * 255.0) as u32, ((1.0 - rain) * 255.0) as u32);
                    [pixel[0], pixel[1], pixel[2]]
                })
            })
            .unwrap_or(fallback);
        rgb.map(|channel| channel as f32 / 255.0)
    }
    pub(crate) fn grass(&self, biome: BiomeSample) -> [f32; 3] {
        Self::color(&self.grass, biome.grass_color, biome, [142, 185, 113])
    }
    pub(crate) fn foliage(&self, biome: BiomeSample) -> [f32; 3] {
        Self::color(&self.foliage, biome.foliage_color, biome, [113, 167, 77])
    }
}
/// RedstoneWireBlock.COLORS, including ARGB.colorFromFloat's floor-to-byte step.
pub fn redstone_wire_color_argb(power: u8) -> u32 {
    let level = power.min(15) as f32 / 15.0;
    let red = level * 0.6 + if level > 0.0 { 0.4 } else { 0.3 };
    let green = (level * level * 0.7 - 0.5).clamp(0.0, 1.0);
    let blue = (level * level * 0.6 - 0.7).clamp(0.0, 1.0);
    let byte = |channel: f32| (channel * 255.0).floor() as u32;
    0xff00_0000 | byte(red) << 16 | byte(green) << 8 | byte(blue)
}
fn block_face_tint(
    block: &Block,
    tinted: bool,
    biome: BiomeSample,
    source: &BiomeTint,
) -> [f32; 3] {
    if block.id.path == "water" {
        return biome.water_color.map(|channel| channel as f32 / 255.0);
    }
    if block.id.path == "redstone_wire" && tinted {
        let power = block
            .properties
            .get("power")
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(0);
        let color = redstone_wire_color_argb(power);
        return [
            ((color >> 16) & 255) as f32 / 255.0,
            ((color >> 8) & 255) as f32 / 255.0,
            (color & 255) as f32 / 255.0,
        ];
    }
    if block.id.path == "oak_leaves" {
        source.foliage(biome)
    } else if tinted {
        source.grass(biome)
    } else {
        [1.0; 3]
    }
}
pub fn build<S: Scene>(scene: &S, packs: &PackStack) -> Result<Build> {
    build_internal(scene, packs, true)
}
pub fn build_minimal<S: Scene>(scene: &S, packs: &PackStack) -> Result<Build> {
    build_internal(scene, packs, false)
}
pub fn build_item_model(packs: &PackStack, item: &ResourceId) -> Result<Option<Build>> {
    let Some(model) = crate::model::resolve_item_model(packs, item)? else {
        return Ok(None);
    };
    if model.elements.is_empty() {
        return Ok(None);
    }
    let tints = packs
        .item_definition(item)?
        .map(|value| crate::interface::item_model_tints(packs, &value["model"]))
        .transpose()?
        .unwrap_or_default();
    let textures = model
        .elements
        .iter()
        .flat_map(|element| element.faces.iter())
        .map(|face| face.texture.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let atlas = make_atlas(packs, textures)?;
    let mut mesh = ChunkMesh::default();
    for element in &model.elements {
        for face in &element.faces {
            let corners = element_corners(element, &face.direction)?;
            let [u0, v0, u1, v1] = atlas.region(&face.texture);
            let [a, b, c, d] = face.uv;
            let uv = [
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * b],
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * b],
            ];
            let tint = face
                .tint_index
                .and_then(|index| tints.get(index))
                .copied()
                .unwrap_or([255; 3]);
            let shade = match face.direction.as_str() {
                "up" => 1.0,
                "down" => 0.5,
                "north" | "south" => 0.8,
                _ => 0.6,
            };
            let start = mesh.vertices.len() as u32;
            for (position, uv) in corners.into_iter().zip(uv) {
                mesh.vertices.push(Vertex {
                    position,
                    uv,
                    color: [
                        shade * tint[0] as f32 / 255.0,
                        shade * tint[1] as f32 / 255.0,
                        shade * tint[2] as f32 / 255.0,
                        1.0,
                    ],
                    sky_light: 15.0,
                    block_light: 0.0,
                });
            }
            mesh.indices.extend_from_slice(&[
                start,
                start + 1,
                start + 2,
                start,
                start + 2,
                start + 3,
            ]);
            mesh.faces += 1;
        }
    }
    Ok(Some(Build {
        atlas: Arc::new(atlas),
        chunks: BTreeMap::from([((0, 0), mesh)]),
        sky_light: None,
        scene_revision: 0,
        pack_generation: packs.generation,
    }))
}
fn build_internal<S: Scene>(scene: &S, packs: &PackStack, preload_blocks: bool) -> Result<Build> {
    let mut models: HashMap<String, Vec<(ResolvedModel, u32)>> = HashMap::new();
    let mut textures = BTreeMap::<ResourceId, ()>::new();
    // A broken chest can remain as a dropped item after its block mesh is gone.
    for name in ["entity/enderdragon/dragon_eyes", "entity/enderdragon/dragon", "entity/blaze/blaze", "entity/end_crystal/end_crystal", "entity/end_portal/end_portal", "entity/enderdragon/dragon_fireball", "item/ender_eye"] {
        textures.insert(ResourceId::parse(&format!("minecraft:{name}"))?, ());
    }
    textures.insert(ResourceId::parse("minecraft:entity/chest/normal")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/bat/bat")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/zombie/zombie")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/player/wide/steve")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/skeleton/skeleton")?, ());
    for skin in ["stray", "stray_overlay", "bogged", "bogged_overlay", "parched"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/skeleton/{skin}"))?, ());
    }
    textures.insert(ResourceId::parse("minecraft:entity/creeper/creeper")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/experience/experience_orb")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/spider/spider")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/slime/slime")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/enderman/enderman")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/enderman/enderman_eyes")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/witch/witch")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/iron_golem/iron_golem")?, ());
    for cracks in ["low", "medium", "high"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/iron_golem/iron_golem_crackiness_{cracks}"))?, ());
    }
    // Wolves: each variant's wild, angry and tame looks, adult and baby,
    // and the collars.
    for variant in minecraftoss_entities::wolf::VARIANTS {
        let name = if variant == "pale" { "wolf".to_owned() } else { format!("wolf_{variant}") };
        for state in ["", "_angry", "_tame"] {
            for age in ["", "_baby"] {
                textures.insert(ResourceId::parse(&format!("minecraft:entity/wolf/{name}{state}{age}"))?, ());
            }
        }
    }
    textures.insert(ResourceId::parse("minecraft:entity/wolf/wolf_collar")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/wolf/wolf_collar_baby")?, ());
    // Thrown splash potions: the bottle over its tinted contents.
    textures.insert(ResourceId::parse("minecraft:item/splash_potion")?, ());
    textures.insert(ResourceId::parse("minecraft:item/potion_overlay")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/zombie/husk")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/zombie/husk_baby")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/zombie_villager/zombie_villager")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/zombie_villager/zombie_villager_baby")?, ());
    for kind in ["desert", "jungle", "plains", "savanna", "snow", "swamp", "taiga"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/zombie_villager/type/{kind}"))?, ());
        textures.insert(ResourceId::parse(&format!("minecraft:entity/zombie_villager/baby/{kind}"))?, ());
    }
    for profession in ["armorer", "butcher", "cartographer", "cleric", "farmer", "fisherman", "fletcher", "leatherworker", "librarian", "mason", "nitwit", "shepherd", "toolsmith", "weaponsmith"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/zombie_villager/profession/{profession}"))?, ());
    }
    textures.insert(ResourceId::parse("minecraft:entity/spider/spider_eyes")?, ());
    textures.insert(ResourceId::parse("minecraft:entity/zombie/drowned")?, ());
    textures.insert(
        ResourceId::parse("minecraft:entity/zombie/drowned_baby")?,
        (),
    );
    for path in ["villager", "villager_baby"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/villager/{path}"))?, ());
    }
    for kind in ["desert", "jungle", "plains", "savanna", "snow", "swamp", "taiga"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/villager/type/{kind}"))?, ());
        textures.insert(ResourceId::parse(&format!("minecraft:entity/villager/baby/{kind}"))?, ());
    }
    for profession in ["armorer", "butcher", "cartographer", "cleric", "farmer", "fisherman", "fletcher", "leatherworker", "librarian", "mason", "nitwit", "shepherd", "toolsmith", "weaponsmith"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/villager/profession/{profession}"))?, ());
    }
    for level in ["stone", "iron", "gold", "emerald", "diamond"] {
        textures.insert(ResourceId::parse(&format!("minecraft:entity/villager/profession_level/{level}"))?, ());
    }
    textures.insert(
        ResourceId::parse("minecraft:entity/zombie/zombie_baby")?,
        (),
    );
    for kind in ["temperate", "warm", "cold"] {
        for suffix in ["", "_baby"] {
            textures.insert(
                ResourceId::parse(&format!("minecraft:entity/cow/cow_{kind}{suffix}"))?,
                (),
            );
            textures.insert(
                ResourceId::parse(&format!("minecraft:entity/pig/pig_{kind}{suffix}"))?,
                (),
            );
            textures.insert(
                ResourceId::parse(&format!("minecraft:entity/chicken/chicken_{kind}{suffix}"))?,
                (),
            );
        }
    }
    for kind in ["red", "brown"] {
        for suffix in ["", "_baby"] {
            textures.insert(
                ResourceId::parse(&format!("minecraft:entity/cow/mooshroom_{kind}{suffix}"))?,
                (),
            );
        }
        textures.insert(
            ResourceId::parse(&format!("minecraft:block/{kind}_mushroom"))?,
            (),
        );
    }
    for name in [
        "sheep",
        "sheep_baby",
        "sheep_wool",
        "sheep_wool_baby",
        "sheep_wool_undercoat",
    ] {
        textures.insert(
            ResourceId::parse(&format!("minecraft:entity/sheep/{name}"))?,
            (),
        );
    }
    textures.insert(
        ResourceId::parse("minecraft:entity/equipment/pig_saddle/saddle")?,
        (),
    );
    for id in crate::horse_render::all_textures() {
        textures.insert(id, ());
    }
    // Humanoid armour (`EquipmentLayerRenderer`'s humanoid layers).
    for layer in ["humanoid", "humanoid_leggings"] {
        for material in crate::armor_render::MATERIALS.iter().copied().chain(["leather_overlay"]) {
            let id = ResourceId::parse(&format!("minecraft:entity/equipment/{layer}/{material}"))?;
            if packs.texture(&id)?.is_some() {
                textures.insert(id, ());
            }
        }
    }
    for chunk in scene.chunks() {
        for x in chunk.0 * 16..chunk.0 * 16 + 16 {
            for z in chunk.1 * 16..chunk.1 * 16 + 16 {
                for y in scene.vertical_range() {
                    let Some(block) = scene.block((x, y, z)) else {
                        continue;
                    };
                    let key = model_key(block);
                    if !models.contains_key(&key) {
                        let variants = resolve_block_variants(packs, block)
                            .with_context(|| format!("block {} at {x},{y},{z}", block.id.key()))?;
                        for (model, _) in &variants {
                            for element in &model.elements {
                                for face in &element.faces {
                                    textures.insert(face.texture.clone(), ());
                                }
                            }
                        }
                        models.insert(key, variants);
                    }
                }
            }
        }
    }
    // Reserve block and item sprites up front. Neither placing a new block
    // nor tossing an item should resize/reupload the atlas during gameplay.
    if preload_blocks {
        for prefix in ["textures/block/", "textures/item/"] {
            for path in packs.list("minecraft", prefix)? {
                if let Some(name) = path
                    .strip_prefix("assets/minecraft/textures/")
                    .and_then(|name| name.strip_suffix(".png"))
                {
                    textures.insert(ResourceId::parse(&format!("minecraft:{name}"))?, ());
                }
            }
        }
        for index in 0..12 {
            let id = ResourceId::parse(&format!("minecraft:particle/leaf_{index}"))?;
            textures.insert(id, ());
        }
        for index in 0..4 {
            textures.insert(
                ResourceId::parse(&format!("minecraft:particle/splash_{index}"))?,
                (),
            );
        }
        for index in 0..8 {
            textures.insert(
                ResourceId::parse(&format!("minecraft:particle/generic_{index}"))?,
                (),
            );
        }
        textures.insert(ResourceId::parse("minecraft:particle/note")?, ());
        textures.insert(ResourceId::parse("minecraft:particle/glow")?, ());
        textures.insert(ResourceId::parse("minecraft:misc/shadow")?, ());
    }
    let atlas = make_atlas(packs, textures.into_keys().collect())?;
    let tint = BiomeTint::from_pack(packs)?;
    let light = SkyLight::build(scene);
    let mut chunks = BTreeMap::new();
    for chunk in scene.chunks() {
        chunks.insert(
            chunk,
            mesh_chunk(scene, chunk, &models, &atlas, &tint, &light)?,
        );
    }
    Ok(Build {
        atlas: Arc::new(atlas),
        chunks,
        sky_light: Some(Arc::new(light)),
        scene_revision: scene.revision(),
        pack_generation: packs.generation,
    })
}
/// Rebuild the chunks whose face visibility or 15-block skylight neighborhood
/// can change after a single block edit, using the existing texture atlas.
/// A newly introduced texture requests an explicit atlas rebuild instead of
/// silently drawing a missing-texture face.
pub fn rebuild_near<S: Scene>(
    scene: &S,
    packs: &PackStack,
    build: &Build,
    pos: BlockPos,
) -> Result<BTreeMap<ChunkPos, ChunkMesh>> {
    rebuild_near_many_with_previous_light(
        scene,
        packs,
        &build.atlas,
        &build.chunks.keys().copied().collect::<Vec<_>>(),
        &[pos],
        build.sky_light.as_deref(),
    )
    .map(|(chunks, _)| chunks)
}
pub fn rebuild_near_with_atlas<S: Scene>(
    scene: &S,
    packs: &PackStack,
    atlas: &Atlas,
    old_chunks: &[ChunkPos],
    pos: BlockPos,
) -> Result<(BTreeMap<ChunkPos, ChunkMesh>, SkyLight)> {
    rebuild_near_many_with_atlas(scene, packs, atlas, old_chunks, &[pos])
}
pub fn rebuild_near_many_with_atlas<S: Scene>(
    scene: &S,
    packs: &PackStack,
    atlas: &Atlas,
    old_chunks: &[ChunkPos],
    positions: &[BlockPos],
) -> Result<(BTreeMap<ChunkPos, ChunkMesh>, SkyLight)> {
    rebuild_near_many_with_previous_light(scene, packs, atlas, old_chunks, positions, None)
}

pub fn rebuild_near_many_with_previous_light<S: Scene>(
    scene: &S,
    packs: &PackStack,
    atlas: &Atlas,
    old_chunks: &[ChunkPos],
    positions: &[BlockPos],
    previous_light: Option<&SkyLight>,
) -> Result<(BTreeMap<ChunkPos, ChunkMesh>, SkyLight)> {
    let mut chunks = scene.chunks();
    chunks.extend(old_chunks.iter().copied());
    chunks.sort_unstable();
    chunks.dedup();
    let affected = chunks
        .into_iter()
        .filter(|&(cx, cz)| {
            positions.iter().any(|pos| {
                let min_x = cx * 16;
                let min_z = cz * 16;
                min_x <= pos.0 + 15
                    && min_x + 15 >= pos.0 - 15
                    && min_z <= pos.2 + 15
                    && min_z + 15 >= pos.2 - 15
            })
        })
        .collect::<Vec<_>>();
    let mut models = HashMap::new();
    for &(cx, cz) in &affected {
        for x in cx * 16..cx * 16 + 16 {
            for z in cz * 16..cz * 16 + 16 {
                for y in scene.vertical_range() {
                    let Some(block) = scene.block((x, y, z)) else {
                        continue;
                    };
                    let key = model_key(block);
                    if models.contains_key(&key) {
                        continue;
                    }
                    let variants = resolve_block_variants(packs, block)
                        .with_context(|| format!("block {} at {x},{y},{z}", block.id.key()))?;
                    for (model, _) in &variants {
                        for element in &model.elements {
                            for face in &element.faces {
                                if !atlas.contains(&face.texture) {
                                    return Err(anyhow!("atlas missing {}", face.texture.key()));
                                }
                            }
                        }
                    }
                    models.insert(key, variants);
                }
            }
        }
    }
    let tint = BiomeTint::from_pack(packs)?;
    let light = previous_light.map_or_else(
        || SkyLight::build(scene),
        |old| old.updated(scene, positions),
    );
    let rebuilt: BTreeMap<ChunkPos, ChunkMesh> = affected
        .into_iter()
        .map(|chunk| {
            Ok((
                chunk,
                mesh_chunk(scene, chunk, &models, atlas, &tint, &light)?,
            ))
        })
        .collect::<Result<_>>()?;
    Ok((rebuilt, light))
}

/// Pack-resolved item models are cached, while entity positions, bob, and
/// rotation are rebuilt at frame rate from interpolated 20 Hz physics state.
#[derive(Default)]
pub struct ItemVisuals {
    models: HashMap<String, ItemVisual>,
}
struct ItemVisual {
    model: ResolvedModel,
    ground: Mat4,
    /// The `thirdperson_righthand` display transform.
    right_hand: Mat4,
    first_person: Mat4,
    min_y: f32,
    tints: Vec<[f32; 3]>,
    flat: bool,
}
fn face_normal(direction: &str) -> Result<Vec3> {
    let (x, y, z) = self::direction(direction)?;
    Ok(Vec3::new(x as f32, y as f32, z as f32))
}
fn level_item_shade(normal: Vec3) -> f32 {
    // entity.vsh uses Lighting.Entry.LEVEL and minecraft_mix_light for
    // dropped item quads, in addition to the sampled world lightmap.
    let light0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let light1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    ((normal.dot(light0).max(0.0) + normal.dot(light1).max(0.0)) * 0.6 + 0.4).min(1.0)
}
impl ItemVisuals {
    pub fn clear(&mut self) {
        self.models.clear();
    }
    fn model<'a>(&'a mut self, id: &str, packs: &PackStack) -> Result<Option<&'a ItemVisual>> {
        if !self.models.contains_key(id) {
            let resource = ResourceId::parse(id)?;
            let Some(model) = crate::model::resolve_item_model(packs, &resource)? else {
                return Ok(None);
            };
            if model.elements.is_empty() {
                return Ok(None);
            }
            let ground = crate::model::item_display_transform(packs, &resource, "ground")?;
            let right_hand = crate::model::item_display_transform(packs, &resource, "thirdperson_righthand")?;
            let definition = packs.item_definition(&resource)?;
            let tints = definition
                .as_ref()
                .map(|value| crate::interface::item_model_tints(packs, &value["model"]))
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .map(|rgb| rgb.map(|component| component as f32 / 255.0))
                .collect();
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for element in &model.elements {
                for x in [element.from[0], element.to[0]] {
                    for y in [element.from[1], element.to[1]] {
                        for z in [element.from[2], element.to[2]] {
                            let point = ground.transform_point3(Vec3::new(x, y, z));
                            min = min.min(point);
                            max = max.max(point);
                        }
                    }
                }
            }
            self.models.insert(
                id.into(),
                ItemVisual {
                    model,
                    ground,
                    right_hand,
                    first_person: crate::model::item_first_person_transform(packs, &resource)?,
                    min_y: min.y,
                    tints,
                    flat: max.z - min.z <= 0.0625,
                },
            );
        }
        Ok(self.models.get(id))
    }
    pub fn mesh(
        &mut self,
        items: &WorldItems,
        packs: &PackStack,
        atlas: &Atlas,
        sky_light: &SkyLight,
        partial_tick: f32,
    ) -> Result<ChunkMesh> {
        let mut mesh = ChunkMesh::default();
        for (entity, pickup_position) in items.entities.iter().map(|entity| (entity, None)).chain(
            items
                .pickup_effects
                .iter()
                .map(|effect| (&effect.item, Some(effect.position(partial_tick)))),
        ) {
            let Some(visual) = self.model(&entity.stack.id, packs)? else {
                continue;
            };
            let partial = partial_tick.clamp(0.0, 1.0);
            let pos = pickup_position
                .unwrap_or_else(|| {
                    entity
                        .previous_position
                        .lerp(entity.position, partial as f64)
                })
                .as_vec3();
            let light_pos = (
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            );
            let sky = sky_light.get(light_pos) as f32;
            let age = entity.age.saturating_sub(1) as f32 + partial;
            let bob = (age / 10.0 + entity.bob_offset).sin() * 0.1 + 0.1;
            let spin = age / 20.0 + entity.bob_offset;
            let copies = match entity.stack.count {
                0 | 1 => 1,
                2..=16 => 2,
                17..=32 => 3,
                33..=48 => 4,
                _ => 5,
            };
            for copy in 0..copies {
                let offset = if copy == 0 {
                    Vec3::ZERO
                } else {
                    let seed = (entity.bob_offset.to_bits() as u64)
                        .wrapping_add((copy as u64).wrapping_mul(0x9e3779b97f4a7c15));
                    let random = |shift: u32| {
                        ((seed.rotate_left(shift) as u32) as f32 / u32::MAX as f32 * 2.0 - 1.0)
                            * 0.15
                    };
                    Vec3::new(
                        random(7),
                        random(19),
                        if visual.flat { 0.0 } else { random(31) },
                    )
                };
                let pose = Mat4::from_translation(pos + Vec3::Y * (bob - visual.min_y + 0.0625))
                    * Mat4::from_rotation_y(spin)
                    * Mat4::from_translation(offset)
                    * visual.ground;
                let normal_pose = pose.inverse().transpose();
                for element in &visual.model.elements {
                    for face in &element.faces {
                        if !atlas.contains(&face.texture) {
                            continue;
                        }
                        let corners = element_corners(element, &face.direction)?;
                        let [u0, v0, u1, v1] = atlas.region(&face.texture);
                        let [a, b, c, d] = face.uv;
                        let uv = [
                            [u0 + (u1 - u0) * a, v0 + (v1 - v0) * b],
                            [u0 + (u1 - u0) * a, v0 + (v1 - v0) * d],
                            [u0 + (u1 - u0) * c, v0 + (v1 - v0) * d],
                            [u0 + (u1 - u0) * c, v0 + (v1 - v0) * b],
                        ];
                        let tint = face
                            .tint_index
                            .and_then(|index| visual.tints.get(index))
                            .copied()
                            .unwrap_or([1.0; 3]);
                        let normal = face_normal(&face.direction)?;
                        let rotated_normal =
                            Mat4::from_rotation_y((element.rotation_y as f32).to_radians())
                                .transform_vector3(normal);
                        let shade = level_item_shade(
                            normal_pose.transform_vector3(rotated_normal).normalize(),
                        );
                        let start = mesh.vertices.len() as u32;
                        for (corner, uv) in corners.into_iter().zip(uv) {
                            mesh.vertices.push(Vertex {
                                position: pose
                                    .transform_point3(Vec3::from_array(corner))
                                    .to_array(),
                                uv,
                                color: [tint[0] * shade, tint[1] * shade, tint[2] * shade, 1.0],
                                sky_light: sky,
                                block_light: sky_light.get_block(light_pos) as f32,
                            });
                        }
                        mesh.indices.extend_from_slice(&[
                            start,
                            start + 1,
                            start + 2,
                            start,
                            start + 2,
                            start + 3,
                        ]);
                        mesh.faces += 1;
                    }
                }
            }
        }
        Ok(mesh)
    }
}

impl ItemVisuals {
    /// `TntRenderer`: primed TNT as the TNT block, swelling over the last
    /// ten ticks of its fuse and flashing white on alternate five-tick
    /// windows. Each entry is an interpolated position and the remaining
    /// fuse (`fuse - partialTick + 1`). The white overlay mixes 75% white
    /// into the texture in vanilla; here the vertex colour brightens it.
    pub fn append_primed_tnt(
        &mut self,
        mesh: &mut ChunkMesh,
        tnt: &[(Vec3, f32)],
        packs: &PackStack,
        atlas: &Atlas,
        sky_light: &SkyLight,
    ) -> Result<()> {
        for &(pos, fuse) in tnt {
            let mut scale = 1.0;
            if fuse < 10.0 {
                let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                let g = g * g;
                scale = 1.0 + g * g * 0.3;
            }
            let white = (fuse as i32) / 5 % 2 == 0;
            let pose = Mat4::from_translation(pos + Vec3::Y * 0.5)
                * Mat4::from_scale(Vec3::splat(scale))
                * Mat4::from_rotation_y((-90.0f32).to_radians())
                * Mat4::from_translation(Vec3::new(-0.5, -0.5, 0.5))
                * Mat4::from_rotation_y(90.0f32.to_radians());
            self.append_block_model(mesh, "minecraft:tnt", pos, pose, if white { 4.0 } else { 1.0 }, packs, atlas, sky_light)?;
        }
        Ok(())
    }

    /// `FallingBlockRenderer`: each falling block's model, shifted so the
    /// block is centred on the entity's feet. Items stand in for block
    /// models (`id` is the block's item).
    pub fn append_falling_blocks(&mut self, mesh: &mut ChunkMesh, blocks: &[(Vec3, String)], packs: &PackStack, atlas: &Atlas, sky_light: &SkyLight) -> Result<()> {
        for (pos, id) in blocks {
            let pose = Mat4::from_translation(*pos + Vec3::new(-0.5, 0.0, -0.5));
            self.append_block_model(mesh, id, *pos, pose, 1.0, packs, atlas, sky_light)?;
        }
        Ok(())
    }

    /// Items mobs hold (`ItemInHandLayer`, `CrossedArmsItemLayer`): each
    /// under its holder's pose, then its display transform, tinted as its
    /// item definition says, lit at its point.
    pub fn append_held_items(&mut self, mesh: &mut ChunkMesh, items: &[HeldItem], packs: &PackStack, atlas: &Atlas, sky_light: &SkyLight) -> Result<()> {
        for item in items {
            let Some((display, mut tints)) = self.model(&item.id, packs)?.map(|visual| {
                (match item.display { HeldDisplay::FirstPerson => visual.first_person, HeldDisplay::RightHand => visual.right_hand, HeldDisplay::Ground => visual.ground }, visual.tints.clone())
            }) else {
                continue;
            };
            if let Some(color) = item.first_tint {
                if tints.is_empty() {
                    tints.push(color);
                } else {
                    tints[0] = color;
                }
            }
            self.append_model(mesh, &item.id, item.light, item.pose * display, 1.0, Some(&tints), packs, atlas, sky_light)?;
        }
        Ok(())
    }

    /// Block models under whole poses (an enderman's carried block), each lit
    /// at its point.
    pub fn append_posed_blocks(&mut self, mesh: &mut ChunkMesh, blocks: &[(Mat4, Vec3, String)], packs: &PackStack, atlas: &Atlas, sky_light: &SkyLight) -> Result<()> {
        for (pose, light, id) in blocks {
            self.append_block_model(mesh, id, *light, *pose, 1.0, packs, atlas, sky_light)?;
        }
        Ok(())
    }

    /// One item model placed with a pose, lit at `pos`, untinted.
    #[allow(clippy::too_many_arguments)]
    fn append_block_model(&mut self, mesh: &mut ChunkMesh, id: &str, pos: Vec3, pose: Mat4, brightness: f32, packs: &PackStack, atlas: &Atlas, sky_light: &SkyLight) -> Result<()> {
        self.append_model(mesh, id, pos, pose, brightness, None, packs, atlas, sky_light)
    }

    /// One item model placed with a pose, lit at `pos`, its faces with a
    /// tint index coloured from `tints` when given.
    #[allow(clippy::too_many_arguments)]
    fn append_model(&mut self, mesh: &mut ChunkMesh, id: &str, pos: Vec3, pose: Mat4, brightness: f32, tints: Option<&[[f32; 3]]>, packs: &PackStack, atlas: &Atlas, sky_light: &SkyLight) -> Result<()> {
        let Some(visual) = self.model(id, packs)? else {
            return Ok(());
        };
        {
            let white = brightness > 1.0;
            let light_pos = (pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
            let sky = sky_light.get(light_pos) as f32;
            let block = sky_light.get_block(light_pos) as f32;
            for element in &visual.model.elements {
                for face in &element.faces {
                    if !atlas.contains(&face.texture) {
                        continue;
                    }
                    let corners = element_corners(element, &face.direction)?;
                    let [u0, v0, u1, v1] = atlas.region(&face.texture);
                    let [a, b, c, d] = face.uv;
                    let uv = [
                        [u0 + (u1 - u0) * a, v0 + (v1 - v0) * b],
                        [u0 + (u1 - u0) * a, v0 + (v1 - v0) * d],
                        [u0 + (u1 - u0) * c, v0 + (v1 - v0) * d],
                        [u0 + (u1 - u0) * c, v0 + (v1 - v0) * b],
                    ];
                    let normal = pose.transform_vector3(face_normal(&face.direction)?).normalize();
                    let shade = level_item_shade(normal) * if white { 4.0 } else { 1.0 };
                    let tint = tints.zip(face.tint_index).and_then(|(tints, index)| tints.get(index)).copied().unwrap_or([1.0; 3]);
                    let start = mesh.vertices.len() as u32;
                    for (corner, uv) in corners.into_iter().zip(uv) {
                        mesh.vertices.push(Vertex {
                            position: pose.transform_point3(Vec3::from_array(corner)).to_array(),
                            uv,
                            color: [tint[0] * shade, tint[1] * shade, tint[2] * shade, 1.0],
                            sky_light: sky,
                            block_light: block,
                        });
                    }
                    mesh.indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
                    mesh.faces += 1;
                }
            }
        }
        Ok(())
    }
}

/// Which display transform a held item takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldDisplay {
    FirstPerson,
    /// `THIRD_PERSON_RIGHT_HAND` (`ItemInHandLayer`).
    RightHand,
    /// `GROUND` (`CrossedArmsItemLayer`, a witch's potion).
    Ground,
}

/// An item a mob holds: its holder's pose, where it is lit, the item, its
/// display and a colour for its first tint (a potion's).
#[derive(Clone, Debug)]
pub struct HeldItem {
    pub pose: Mat4,
    pub light: Vec3,
    pub id: String,
    pub display: HeldDisplay,
    pub first_tint: Option<[f32; 3]>,
}

/// What `EntityRenderer.extractShadow` reads of one entity.
#[derive(Clone, Copy, Debug)]
pub struct ShadowCaster {
    /// The render state's position: the interpolated feet.
    pub position: glam::DVec3,
    /// `distanceToCameraSq`: from the entity's current position.
    pub distance_squared: f64,
    /// `getShadowRadius`, before the 32-block cap.
    pub radius: f32,
    /// `getShadowStrength`.
    pub strength: f32,
}

/// Dropped items' shadows (`ItemEntityRenderer`: radius 0.15, strength
/// 0.75).
pub fn item_shadow_casters(items: &WorldItems, camera_position: glam::DVec3, partial_tick: f32) -> Vec<ShadowCaster> {
    items
        .entities
        .iter()
        .map(|entity| ShadowCaster {
            position: entity.previous_position.lerp(entity.position, f64::from(partial_tick.clamp(0.0, 1.0))),
            distance_squared: entity.position.distance_squared(camera_position),
            radius: 0.15,
            strength: 0.75,
        })
        .collect()
}

/// `EntityRenderer.extractShadow`/`extractShadowPiece` and
/// `ShadowFeatureRenderer`: each block column under the entity within its
/// radius, down to a depth its strength allows, gets the shadow texture
/// on top of a full-collision, visible block below a spot lit above 3,
/// fading with depth and darkening with that light (`Lightmap.
/// getBrightness`). `raw_brightness` is `getMaxLocalRawBrightness`; the
/// texture clamps outside the circle, so each quad is cut to the radius.
pub fn entity_shadows<S: Scene>(
    casters: &[ShadowCaster],
    scene: &S,
    atlas: &Atlas,
    raw_brightness: &dyn Fn(BlockPos) -> u8,
    ambient_light: f32,
) -> Result<ChunkMesh> {
    let mut mesh = ChunkMesh::default();
    let sprite = ResourceId::parse("minecraft:misc/shadow")?;
    if !atlas.contains(&sprite) {
        return Ok(mesh);
    }
    let [u0, v0, u1, v1] = atlas.region(&sprite);
    for caster in casters {
        let radius = caster.radius.min(32.0);
        if radius <= 0.0 {
            continue;
        }
        let pow = ((1.0 - caster.distance_squared / 256.0) * f64::from(caster.strength)) as f32;
        if pow <= 0.0 {
            continue;
        }
        let state = caster.position;
        let r = f64::from(radius);
        let depth = (pow / 0.5 - 1.0).min(radius);
        let (x0, x1) = ((state.x - r).floor() as i32, (state.x + r).floor() as i32);
        let (z0, z1) = ((state.z - r).floor() as i32, (state.z + r).floor() as i32);
        let (y0, y1) = ((state.y - f64::from(depth)).floor() as i32, state.y.floor() as i32);
        for z in z0..=z1 {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    let power_at_depth = pow - (state.y - f64::from(y)) as f32 * 0.5;
                    let below = (x, y - 1, z);
                    // RenderShape.INVISIBLE: air and barriers.
                    if scene.block(below).is_none_or(|b| matches!(b.id.path.as_str(), "air" | "cave_air" | "void_air" | "barrier" | "structure_void" | "light" | "moving_piston")) {
                        continue;
                    }
                    let brightness = raw_brightness((x, y, z));
                    if brightness <= 3 || !scene.full_collision_at(below) {
                        continue;
                    }
                    let v = f32::from(brightness) / 15.0;
                    let curved = v / (4.0 - 3.0 * v);
                    let light = ambient_light * (1.0 - curved) + curved;
                    let alpha = (power_at_depth * 0.5 * light).clamp(0.0, 1.0);
                    // The piece's corners relative to the entity, as floats.
                    let (rx, ry, rz) = ((f64::from(x) - state.x) as f32, (f64::from(y) - state.y) as f32, (f64::from(z) - state.z) as f32);
                    let (x_min, x_max) = (rx.max(-radius), (rx + 1.0).min(radius));
                    let (z_min, z_max) = (rz.max(-radius), (rz + 1.0).min(radius));
                    if x_min >= x_max || z_min >= z_max {
                        continue;
                    }
                    let origin = state.as_vec3();
                    let start = mesh.vertices.len() as u32;
                    for (px, pz) in [(x_min, z_min), (x_min, z_max), (x_max, z_max), (x_max, z_min)] {
                        let su = -px / 2.0 / radius + 0.5;
                        let sv = -pz / 2.0 / radius + 0.5;
                        mesh.vertices.push(Vertex {
                            // Lifted a thousandth off the block top so it
                            // never loses the depth test to it.
                            position: (origin + Vec3::new(px, ry + 0.001, pz)).to_array(),
                            uv: [u0 + (u1 - u0) * su, v0 + (v1 - v0) * sv],
                            color: [1.0, 1.0, 1.0, alpha],
                            sky_light: 15.0,
                            block_light: 0.0,
                        });
                    }
                    mesh.indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
                    mesh.faces += 1;
                }
            }
        }
    }
    Ok(mesh)
}
fn model_key(block: &Block) -> String {
    format!("{}{:?}", block.id.key(), block.properties)
}
/// BlockBehaviour.getSeed -> Mth.getSeed -> SingleThreadedRandomSource.nextInt.
/// WeightedVariants consumes the first draw for the selected model.
fn variant_index(variants: &[(ResolvedModel, u32)], (x, y, z): BlockPos) -> usize {
    if variants.len() == 1 {
        return 0;
    }
    let total: u32 = variants.iter().map(|(_, weight)| *weight).sum();
    if total == 0 {
        return 0;
    }
    let seed = x.wrapping_mul(3_129_871) as i64 ^ (z as i64).wrapping_mul(116_129_781) ^ y as i64;
    let seed = seed
        .wrapping_mul(seed)
        .wrapping_mul(42_317_861)
        .wrapping_add(seed.wrapping_mul(11))
        >> 16;
    let mut random = ((seed as u64) ^ 0x5deece66d) & ((1u64 << 48) - 1);
    let mut next = || {
        random = random.wrapping_mul(0x5deece66d).wrapping_add(11) & ((1u64 << 48) - 1);
        (random >> 17) as u32
    };
    let mut selection = if total.is_power_of_two() {
        ((total as u64 * next() as u64) >> 31) as u32
    } else {
        loop {
            let bits = next();
            let value = bits % total;
            if (bits as i32)
                .wrapping_sub(value as i32)
                .wrapping_add(total as i32 - 1)
                >= 0
            {
                break value;
            }
        }
    };
    for (index, (_, weight)) in variants.iter().enumerate() {
        if selection < *weight {
            return index;
        }
        selection -= *weight;
    }
    variants.len() - 1
}
/// A face's corners as `FaceBakery.bakeVertex` places them: the element's
/// own rotation about its origin, then the blockstate's.
fn element_corners(element: &crate::model::Element, face: &str) -> Result<[[f32; 3]; 4]> {
    Ok(corners(face, element.from, element.to)?.map(|corner| rotate_y(element.rotation.map_or(corner, |r| r.apply(corner)), element.rotation_y)))
}

/// The baked quad's direction (`FaceBakery.calculateFacing`): the face's
/// own, turned with the blockstate, or for a rotated element the direction
/// closest to its normal (`GeometryUtils.normal` of the first three
/// corners; the first of equals in `Direction` order).
fn quad_direction(element: &crate::model::Element, face: &str, corners: &[[f32; 3]; 4]) -> Result<&'static str> {
    if element.rotation.is_none() {
        let dir = rotated_direction(face, element.rotation_y)?;
        return Ok(["down", "up", "north", "south", "west", "east"].into_iter().find(|d| *d == dir).unwrap_or("up"));
    }
    let [v0, v1, v2, _] = *corners;
    let mut n = [
        (v1[1] - v0[1]) * (v2[2] - v0[2]) - (v1[2] - v0[2]) * (v2[1] - v0[1]),
        (v1[2] - v0[2]) * (v2[0] - v0[0]) - (v1[0] - v0[0]) * (v2[2] - v0[2]),
        (v1[0] - v0[0]) * (v2[1] - v0[1]) - (v1[1] - v0[1]) * (v2[0] - v0[0]),
    ];
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !(length > 0.0) || !length.is_finite() {
        return Ok("up");
    }
    for v in &mut n {
        *v /= length;
    }
    let mut best = "up";
    let mut closest = 0.0f32;
    for (name, [x, y, z]) in [("down", [0.0, -1.0, 0.0]), ("up", [0.0, 1.0, 0.0]), ("north", [0.0, 0.0, -1.0]), ("south", [0.0, 0.0, 1.0]), ("west", [-1.0, 0.0, 0.0]), ("east", [1.0, 0.0, 0.0])] {
        let product = n[0] * x + n[1] * y + n[2] * z;
        if product >= 0.0 && product > closest {
            closest = product;
            best = name;
        }
    }
    Ok(best)
}

/// `BlockModelLighter.prepareQuadShape`'s `faceCubic`: the quad lies flat
/// on the block's side it faces (a full collision block's quads count as
/// on their side too; not modelled).
fn face_cubic(dir: &str, corners: &[[f32; 3]; 4]) -> bool {
    let axis = match dir {
        "down" | "up" => 1,
        "north" | "south" => 2,
        _ => 0,
    };
    let (min, max) = corners.iter().fold((32.0f32, -32.0f32), |(lo, hi), c| (lo.min(c[axis]), hi.max(c[axis])));
    min == max && if matches!(dir, "down" | "north" | "west") { min < 1.0e-4 } else { max > 0.9999 }
}

fn rotate_y([x, y, z]: [f32; 3], degrees: u16) -> [f32; 3] {
    match degrees {
        90 => [1.0 - z, y, x],
        180 => [1.0 - x, y, 1.0 - z],
        270 => [z, y, 1.0 - x],
        _ => [x, y, z],
    }
}
fn rotated_direction(dir: &str, degrees: u16) -> Result<&str> {
    let i = match dir {
        "north" => 0,
        "east" => 1,
        "south" => 2,
        "west" => 3,
        "up" | "down" => return Ok(dir),
        _ => return Err(anyhow!("unsupported face direction {dir}")),
    };
    Ok(["north", "east", "south", "west"][(i + degrees as usize / 90) % 4])
}
pub fn make_atlas(packs: &PackStack, mut textures: Vec<ResourceId>) -> Result<Atlas> {
    textures.insert(0, ResourceId::parse("minecraft:missingno")?);
    let tile = 64u32;
    // Entity skins keep their own resolution, as vanilla binds each one on
    // its own: one larger than a cell spans several cells, a texel to a
    // pixel. Everything else fills one cell.
    let mut images = Vec::with_capacity(textures.len());
    let mut spans = Vec::with_capacity(textures.len());
    for id in &textures {
        let bytes = if id.path == "missingno" { None } else { packs.texture(id)? };
        let span = match &bytes {
            Some(bytes) if id.path.starts_with("entity/") && packs.animation(id)?.is_none() => {
                let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png).into_dimensions()?;
                (width.div_ceil(tile).max(1), height.div_ceil(tile).max(1))
            }
            _ => (1, 1),
        };
        images.push(bytes);
        spans.push(span);
    }
    let cells: u32 = spans.iter().map(|&(w, h)| w * h).sum();
    let mut side = (cells as f32).sqrt().ceil() as u32;
    let positions = loop {
        if let Some(positions) = place_cells(&spans, side) {
            break positions;
        }
        side += 1;
    };
    let mut pixels = RgbaImage::new(side * tile, side * tile);
    let mut mipmaps = (1..=4)
        .map(|level| RgbaImage::new(side * (tile >> level), side * (tile >> level)))
        .collect::<Vec<_>>();
    let mut slots = HashMap::new();
    let mut missing = Vec::new();
    let mut animated = Vec::new();
    let mut animated_tiles = Vec::new();
    for (i, id) in textures.into_iter().enumerate() {
        let (col, row) = positions[i];
        let (span_w, span_h) = spans[i];
        let image = images[i].take();
        let mut dark_cutout = false;
        let mut native_leaf_mips = None;
        let tile_image = if let Some(bytes) = image {
            let decoded =
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8();
            let frame = if let Some(meta) = packs.animation(&id)? {
                dark_cutout = meta
                    .get("texture")
                    .and_then(|texture| texture.get("mipmap_strategy"))
                    .and_then(|value| value.as_str())
                    == Some("dark_cutout");
                if let Some(animation) = meta.get("animation") {
                    animated.push(id.clone());
                    if let Some(tile) = animated_tile(&decoded, animation, (col * tile, row * tile))
                    {
                        animated_tiles.push(tile);
                    }
                    let index = animation
                        .get("frames")
                        .and_then(|f| f.as_array())
                        .and_then(|a| a.first())
                        .and_then(|v| {
                            v.as_u64()
                                .or_else(|| v.get("index").and_then(|x| x.as_u64()))
                        })
                        .unwrap_or(0) as u32;
                    let size = decoded.width().min(decoded.height());
                    image::imageops::crop_imm(
                        &decoded,
                        0,
                        (index * size).min(decoded.height() - size),
                        size,
                        size,
                    )
                    .to_image()
                } else {
                    decoded
                }
            } else {
                decoded
            };
            if dark_cutout && frame.width() == 16 && frame.height() == 16 {
                native_leaf_mips = Some(texture_mips::dark_cutout(frame, 5));
                image::imageops::resize(
                    &native_leaf_mips.as_ref().unwrap()[0],
                    tile,
                    tile,
                    FilterType::Nearest,
                )
            } else {
                image::imageops::resize(&frame, span_w * tile, span_h * tile, FilterType::Nearest)
            }
        } else {
            if id.path != "missingno" {
                missing.push(id.clone());
            }
            let mut checker = RgbaImage::new(tile, tile);
            for y in 0..tile {
                for x in 0..tile {
                    checker.put_pixel(
                        x,
                        y,
                        if (x / 8 + y / 8) % 2 == 0 {
                            image::Rgba([255, 0, 255, 255])
                        } else {
                            image::Rgba([0, 0, 0, 255])
                        },
                    );
                }
            }
            checker
        };
        image::imageops::replace(
            &mut pixels,
            &tile_image,
            (col * tile) as i64,
            (row * tile) as i64,
        );
        let tile_mips = if let Some(native) = native_leaf_mips {
            // Keep the 16-pixel foliage cutout chain intact while the 64-pixel
            // atlas also preserves the native 64-pixel chest entity texture.
            let mut levels = vec![tile_image];
            levels.push(image::imageops::resize(
                &native[0],
                32,
                32,
                FilterType::Nearest,
            ));
            levels.extend(native.into_iter().take(3));
            levels
        } else if dark_cutout {
            texture_mips::dark_cutout(tile_image, 5)
        } else {
            (0..5)
                .map(|level| {
                    image::imageops::resize(
                        &tile_image,
                        (span_w * tile) >> level,
                        (span_h * tile) >> level,
                        FilterType::Triangle,
                    )
                })
                .collect::<Vec<_>>()
        };
        // The dark-cutout source also alters transparent RGB at level zero.
        if dark_cutout {
            image::imageops::replace(
                &mut pixels,
                &tile_mips[0],
                (col * tile) as i64,
                (row * tile) as i64,
            );
        }
        for (level, mip) in mipmaps.iter_mut().enumerate() {
            let mip_tile = tile >> (level + 1);
            image::imageops::replace(
                mip,
                &tile_mips[level + 1],
                (col * mip_tile) as i64,
                (row * mip_tile) as i64,
            );
        }
        let width = (side * tile) as f32;
        slots.insert(
            id,
            [
                (col * tile) as f32 / width + 0.5 / width,
                (row * tile) as f32 / width + 0.5 / width,
                ((col + span_w) * tile) as f32 / width - 0.5 / width,
                ((row + span_h) * tile) as f32 / width - 0.5 / width,
            ],
        );
    }
    Ok(Atlas {
        pixels,
        mipmaps,
        slots,
        missing,
        animated,
        animated_tiles,
    })
}
/// Grid cells for each texture's span (columns, rows) in a `side`-cell
/// square: the multi-cell ones first, then the rest in order, each at the
/// first free spot; `None` when they do not fit.
fn place_cells(spans: &[(u32, u32)], side: u32) -> Option<Vec<(u32, u32)>> {
    let mut used = vec![false; (side * side) as usize];
    let mut positions = vec![(0, 0); spans.len()];
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(spans[i].0 * spans[i].1));
    let mut first_free = 0;
    for i in order {
        let (w, h) = spans[i];
        let fits = |col: u32, row: u32, used: &[bool]| {
            col + w <= side && row + h <= side && (row..row + h).all(|r| (col..col + w).all(|c| !used[(r * side + c) as usize]))
        };
        let start = if (w, h) == (1, 1) { first_free } else { 0 };
        let cell = (start..side * side).find(|&cell| fits(cell % side, cell / side, &used))?;
        let (col, row) = (cell % side, cell / side);
        for r in row..row + h {
            for c in col..col + w {
                used[(r * side + c) as usize] = true;
            }
        }
        if (w, h) == (1, 1) {
            first_free = cell + 1;
        }
        positions[i] = (col, row);
    }
    Some(positions)
}
fn animated_tile(
    sheet: &RgbaImage,
    metadata: &serde_json::Value,
    origin: (u32, u32),
) -> Option<AnimatedTile> {
    let square = sheet.width().min(sheet.height());
    let width = metadata
        .get("width")
        .and_then(|v| v.as_u64())
        .unwrap_or(square as u64) as u32;
    let height = metadata
        .get("height")
        .and_then(|v| v.as_u64())
        .unwrap_or(square as u64) as u32;
    if width == 0 || height == 0 || sheet.width() % width != 0 || sheet.height() % height != 0 {
        return None;
    }
    let columns = sheet.width() / width;
    let count = columns * (sheet.height() / height);
    let frametime = metadata
        .get("frametime")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let entries = metadata.get("frames").and_then(|v| v.as_array());
    let sequence = if let Some(entries) = entries {
        entries
            .iter()
            .filter_map(|entry| {
                let index = entry.as_u64().or_else(|| entry.get("index")?.as_u64())? as usize;
                let time = entry
                    .get("time")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(frametime as u64) as u32;
                (index < count as usize && time > 0).then_some((index, time))
            })
            .collect::<Vec<_>>()
    } else {
        (0..count as usize)
            .map(|index| (index, frametime))
            .collect()
    };
    if sequence.len() <= 1 || frametime == 0 {
        return None;
    }
    let frames = (0..count)
        .map(|index| {
            let frame = image::imageops::crop_imm(
                sheet,
                index % columns * width,
                index / columns * height,
                width,
                height,
            )
            .to_image();
            let tile = image::imageops::resize(&frame, 64, 64, FilterType::Nearest);
            (0..5)
                .map(|level| {
                    image::imageops::resize(&tile, 64 >> level, 64 >> level, FilterType::Triangle)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Some(AnimatedTile {
        origin,
        frames,
        sequence,
        interpolate: metadata.get("interpolate").and_then(|v| v.as_bool()) == Some(true),
    })
}
fn mesh_chunk<S: Scene>(
    scene: &S,
    chunk: ChunkPos,
    models: &HashMap<String, Vec<(ResolvedModel, u32)>>,
    atlas: &Atlas,
    tint_source: &BiomeTint,
    light: &SkyLight,
) -> Result<ChunkMesh> {
    let mut mesh = ChunkMesh::default();
    let mut transparent_indices = Vec::new();
    let full_cube_models: HashMap<_, _> = models
        .iter()
        .map(|(key, variants)| (key.clone(), all_full_cubes(variants)))
        .collect();
    for x in chunk.0 * 16..chunk.0 * 16 + 16 {
        for z in chunk.1 * 16..chunk.1 * 16 + 16 {
            for y in scene.vertical_range() {
                let Some(block) = scene.block((x, y, z)) else {
                    continue;
                };
                append_block(
                    scene,
                    (x, y, z),
                    block,
                    || {
                        models
                            .get(&model_key(block))
                            .map(Vec::as_slice)
                            .ok_or_else(|| anyhow!("uncached model"))
                    },
                    |neighbor| {
                        scene.block(neighbor).is_some_and(|b| {
                            hides_shared_face(block, b)
                                || (b.is_opaque()
                                    && full_cube_models
                                        .get(&model_key(b))
                                        .copied()
                                        .unwrap_or(false))
                        })
                    },
                    atlas,
                    tint_source,
                    light,
                    &mut mesh,
                    &mut transparent_indices,
                )?;
            }
        }
    }
    Ok(finish_mesh(mesh, transparent_indices))
}
/// Whether every variant has a full-cube element, so an opaque block with
/// these models hides the faces of its neighbors.
pub(crate) fn all_full_cubes(variants: &[(ResolvedModel, u32)]) -> bool {
    variants.iter().all(|(model, _)| {
        model
            .elements
            .iter()
            .any(|element| element.from == [0.0; 3] && element.to == [1.0; 3])
    })
}
/// Moves the translucent faces after the opaque ones.
/// One model face baked for a block state's variant (vanilla `BakedQuad`):
/// everything [`append_block`] works out per face that does not depend on
/// where the block stands, so a section build only culls, shades, lights
/// and emits it.
#[derive(Clone, Debug)]
pub(crate) struct BakedQuad {
    /// The neighbour offset whose block may hide it, for cullable faces.
    cull: Option<(i32, i32, i32)>,
    /// The neighbour its light comes from.
    light_offset: (i32, i32, i32),
    /// For a full cube's face, the two axes its corner shade samples span.
    ao_axes: Option<[usize; 2]>,
    corners: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    shade: f32,
    tinted: bool,
    transparent: bool,
}

/// How a block's faces are tinted (`block_face_tint`, decided per state).
#[derive(Clone, Copy, Debug)]
pub(crate) enum TintKind {
    Water,
    RedstoneWire([f32; 3]),
    OakLeaves,
    Other,
}

impl TintKind {
    pub(crate) fn of(block: &Block) -> Self {
        match block.id.path.as_str() {
            "water" => Self::Water,
            "redstone_wire" => {
                let power = block.properties.get("power").and_then(|value| value.parse::<u8>().ok()).unwrap_or(0);
                let color = redstone_wire_color_argb(power);
                Self::RedstoneWire([((color >> 16) & 255) as f32 / 255.0, ((color >> 8) & 255) as f32 / 255.0, (color & 255) as f32 / 255.0])
            }
            "oak_leaves" => Self::OakLeaves,
            _ => Self::Other,
        }
    }

    /// `block_face_tint` for a face of this kind.
    fn tint(self, tinted: bool, biome: BiomeSample, source: &BiomeTint) -> [f32; 3] {
        match self {
            Self::Water => biome.water_color.map(|channel| channel as f32 / 255.0),
            Self::RedstoneWire(color) if tinted => color,
            Self::OakLeaves => source.foliage(biome),
            _ if tinted => source.grass(biome),
            _ => [1.0; 3],
        }
    }
}

/// Bakes a resolved model's faces for `block`, as [`append_block`] would
/// place them.
pub(crate) fn bake_quads(block: &Block, model: &ResolvedModel, atlas: &Atlas) -> Result<Vec<BakedQuad>> {
    let mut quads = Vec::new();
    for element in &model.elements {
        for face in &element.faces {
            let cull_dir = rotated_direction(face.cullface.as_deref().unwrap_or(face.direction.as_str()), element.rotation_y)?;
            let cull_delta = direction(cull_dir)?;
            let corners = element_corners(element, &face.direction)?;
            let dir = quad_direction(element, &face.direction, &corners)?;
            let delta = direction(dir)?;
            let [u0, v0, u1, v1] = atlas.region(&face.texture);
            let [a, b, c, d] = face.uv;
            let uv = [
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * b],
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * b],
            ];
            let shade = match element.shade_direction_override.as_deref().unwrap_or(dir) {
                "up" => 1.0,
                "down" => 0.5,
                "north" | "south" => 0.8,
                _ => 0.6,
            };
            let light_offset = if face.cull {
                cull_delta
            } else if face_cubic(dir, &corners) {
                delta
            } else {
                (0, 0, 0)
            };
            let full_cube = element.from == [0.0; 3] && element.to == [1.0; 3] && element.rotation.is_none();
            let ao_axes = full_cube.then(|| match dir {
                "up" | "down" => [0, 2],
                "north" | "south" => [0, 1],
                _ => [1, 2],
            });
            quads.push(BakedQuad {
                cull: face.cull.then_some(cull_delta),
                light_offset,
                ao_axes,
                corners,
                uv,
                shade,
                tinted: face.tint,
                transparent: block.id.path == "water" || face.force_translucent,
            });
        }
    }
    Ok(quads)
}

/// [`append_block`] for a block whose faces are baked: the same vertices,
/// with `ao_occluder(pos)` standing for the block test in
/// [`ambient_vertex`] and `hidden_by` for the cull test.
#[allow(clippy::too_many_arguments)]
pub(crate) fn append_baked<S: Scene>(
    scene: &S,
    (x, y, z): BlockPos,
    quads: &[BakedQuad],
    tint_kind: TintKind,
    waterlogged: bool,
    hidden_by: impl Fn(BlockPos) -> bool,
    ao_occluder: impl Fn(BlockPos) -> bool,
    atlas: &Atlas,
    tint_source: &BiomeTint,
    light: &SkyLight,
    mesh: &mut ChunkMesh,
    transparent_indices: &mut Vec<u32>,
) -> Result<()> {
    let mut biome = None;
    for quad in quads {
        if quad.cull.is_some_and(|(dx, dy, dz)| hidden_by((x + dx, y + dy, z + dz))) {
            continue;
        }
        let biome = *biome.get_or_insert_with(|| scene.biome_at((x, y, z)));
        let tint = tint_kind.tint(quad.tinted, biome, tint_source);
        let start = mesh.vertices.len() as u32;
        let light_pos = (x + quad.light_offset.0, y + quad.light_offset.1, z + quad.light_offset.2);
        let block_light = light.get_block(light_pos) as f32;
        for (corner, uv) in quad.corners.iter().zip(quad.uv) {
            let (ao, sky_light) = match quad.ao_axes {
                Some(axes) => ambient_corner(light, light_pos, axes, *corner, &ao_occluder),
                None => (1.0, light.get(light_pos) as f32),
            };
            mesh.vertices.push(Vertex {
                position: [x as f32 + corner[0], y as f32 + corner[1], z as f32 + corner[2]],
                uv,
                color: [quad.shade * ao * tint[0], quad.shade * ao * tint[1], quad.shade * ao * tint[2], 1.0],
                sky_light,
                block_light,
            });
        }
        let indices = if quad.transparent { &mut *transparent_indices } else { &mut mesh.indices };
        indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
        mesh.faces += 1;
    }
    if waterlogged {
        append_fluid(scene, (x, y, z), atlas, tint_source, light, mesh, transparent_indices)?;
    }
    Ok(())
}

/// [`ambient_vertex`] with its axes worked out and its block test given.
fn ambient_corner(light: &SkyLight, base: (i32, i32, i32), axes: [usize; 2], corner: [f32; 3], occluder: &impl Fn(BlockPos) -> bool) -> (f32, f32) {
    let mut side = [[0; 3]; 2];
    for (i, axis) in axes.into_iter().enumerate() {
        side[i][axis] = if corner[axis] < 0.5 { -1 } else { 1 };
    }
    let positions = [
        base,
        (base.0 + side[0][0], base.1 + side[0][1], base.2 + side[0][2]),
        (base.0 + side[1][0], base.1 + side[1][1], base.2 + side[1][2]),
        (base.0 + side[0][0] + side[1][0], base.1 + side[0][1] + side[1][1], base.2 + side[0][2] + side[1][2]),
    ];
    let center_light = light.get(base);
    let shade = positions.iter().map(|&p| if occluder(p) { 0.2 } else { 1.0 }).sum::<f32>() * 0.25;
    let sky = positions
        .iter()
        .map(|&p| {
            let level = light.get(p);
            if center_light > 2 && level == 0 { center_light as f32 } else { level as f32 }
        })
        .sum::<f32>()
        * 0.25;
    (shade, sky)
}

pub(crate) fn variant_for(variants: &[(ResolvedModel, u32)], pos: BlockPos) -> usize {
    variant_index(variants, pos)
}

pub(crate) fn finish_mesh(mut mesh: ChunkMesh, transparent_indices: Vec<u32>) -> ChunkMesh {
    if !transparent_indices.is_empty() {
        mesh.transparent_start = Some(mesh.indices.len() as u32);
        mesh.indices.extend(transparent_indices);
    }
    mesh
}
/// Appends one block's faces. `variants` supplies its resolved models when
/// the block is not a fluid; `hidden_by(neighbor)` is whether the block at a
/// neighboring position hides a cullable face toward it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn append_block<'m, S: Scene>(
    scene: &S,
    (x, y, z): BlockPos,
    block: &Block,
    variants: impl FnOnce() -> Result<&'m [(ResolvedModel, u32)]>,
    hidden_by: impl Fn(BlockPos) -> bool,
    atlas: &Atlas,
    tint_source: &BiomeTint,
    light: &SkyLight,
    mesh: &mut ChunkMesh,
    transparent_indices: &mut Vec<u32>,
) -> Result<()> {
    if block.id.path == "chest" {
        // Chest block entities have a separately posed lid/lock.
        return Ok(());
    }
    if matches!(block.id.path.as_str(), "water" | "lava") {
        return append_fluid(
            scene,
            (x, y, z),
            atlas,
            tint_source,
            light,
            mesh,
            transparent_indices,
        );
    }
    let variants = variants()?;
    let model = &variants[variant_index(variants, (x, y, z))].0;
    let mut biome = None;
    for element in &model.elements {
        for face in &element.faces {
            // Culled quads go with their cullface (turned with the
            // blockstate), and take their light from that neighbour.
            let cull_dir = rotated_direction(face.cullface.as_deref().unwrap_or(face.direction.as_str()), element.rotation_y)?;
            let cull_delta = direction(cull_dir)?;
            if face.cull && hidden_by((x + cull_delta.0, y + cull_delta.1, z + cull_delta.2)) {
                continue;
            }
            let corners = element_corners(element, &face.direction)?;
            let dir = quad_direction(element, &face.direction, &corners)?;
            let delta = direction(dir)?;
            let [u0, v0, u1, v1] = atlas.region(&face.texture);
            let [a, b, c, d] = face.uv;
            let uv = [
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * b],
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * b],
            ];
            let shade = match element.shade_direction_override.as_deref().unwrap_or(dir) {
                "up" => 1.0,
                "down" => 0.5,
                "north" | "south" => 0.8,
                _ => 0.6,
            };
            let biome = *biome.get_or_insert_with(|| scene.biome_at((x, y, z)));
            let tint = block_face_tint(block, face.tint, biome, tint_source);
            let alpha = if block.id.path == "water" { 0.72 } else { 1.0 };
            let start = mesh.vertices.len() as u32;
            // `ModelBlockRenderer.tesselateFlat` and `prepareQuadFlat`: a
            // culled quad is lit from its cullface's neighbour, another from
            // the neighbour it faces only when it lies on that side
            // (`faceCubic`), else from its own block.
            let light_pos = if face.cull {
                (x + cull_delta.0, y + cull_delta.1, z + cull_delta.2)
            } else if face_cubic(dir, &corners) {
                (x + delta.0, y + delta.1, z + delta.2)
            } else {
                (x, y, z)
            };
            let full_cube = element.from == [0.0; 3] && element.to == [1.0; 3] && element.rotation.is_none();
            for (corner, uv) in corners.iter().zip(uv) {
                let (ao, sky_light) = if full_cube {
                    ambient_vertex(scene, light, light_pos, dir, *corner)
                } else {
                    (1.0, light.get(light_pos) as f32)
                };
                mesh.vertices.push(Vertex {
                    position: [
                        x as f32 + corner[0],
                        y as f32 + corner[1],
                        z as f32 + corner[2],
                    ],
                    uv,
                    color: [
                        shade * ao * tint[0],
                        shade * ao * tint[1],
                        shade * ao * tint[2],
                        alpha,
                    ],
                    sky_light,
                    block_light: light.get_block(light_pos) as f32,
                });
            }
            let indices = if block.id.path == "water" || face.force_translucent {
                &mut *transparent_indices
            } else {
                &mut mesh.indices
            };
            indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
            mesh.faces += 1;
        }
    }
    if block
        .properties
        .get("waterlogged")
        .is_some_and(|value| value == "true")
    {
        append_fluid(
            scene,
            (x, y, z),
            atlas,
            tint_source,
            light,
            mesh,
            transparent_indices,
        )?;
    }
    Ok(())
}
fn fluid_render_height<S: Scene>(scene: &S, kind: FluidKind, pos: BlockPos) -> f32 {
    if let Some(cell) = FluidCell::at(scene, pos).filter(|cell| cell.kind == kind) {
        cell.height(scene, pos)
    } else if scene.full_collision_at(pos) {
        -1.0
    } else {
        0.0
    }
}
fn fluid_corner_height<S: Scene>(
    scene: &S,
    kind: FluidKind,
    own: f32,
    a: f32,
    b: f32,
    corner: BlockPos,
) -> f32 {
    if a >= 1.0 || b >= 1.0 {
        return 1.0;
    }
    let mut heights = vec![own, a, b];
    if a > 0.0 || b > 0.0 {
        heights.push(fluid_render_height(scene, kind, corner));
    }
    let (sum, weight) =
        heights
            .into_iter()
            .filter(|h| *h >= 0.0)
            .fold((0.0, 0.0), |(sum, weight), h| {
                let w = if h >= 0.8 { 10.0 } else { 1.0 };
                (sum + h * w, weight + w)
            });
    if weight == 0.0 {
        0.0
    } else {
        sum / weight
    }
}
fn fluid_quad(
    mesh: &mut ChunkMesh,
    transparent: &mut Vec<u32>,
    vertices: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    color: [f32; 4],
    sky: f32,
    block_light: f32,
    water: bool,
) {
    let start = mesh.vertices.len() as u32;
    for (position, uv) in vertices.into_iter().zip(uv) {
        mesh.vertices.push(Vertex {
            position,
            uv,
            color,
            sky_light: sky,
            block_light,
        });
    }
    let indices = if water {
        transparent
    } else {
        &mut mesh.indices
    };
    indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    mesh.faces += 1;
}
thread_local! {
    /// Checks only: fluid with no faces showing goes the whole way through
    /// [`append_fluid`] instead of stopping early.
    pub(crate) static FLUID_FULL_PATH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn append_fluid<S: Scene>(
    scene: &S,
    pos: BlockPos,
    atlas: &Atlas,
    tint_source: &BiomeTint,
    light: &SkyLight,
    mesh: &mut ChunkMesh,
    transparent: &mut Vec<u32>,
) -> Result<()> {
    let cell = FluidCell::at(scene, pos).expect("fluid block has fluid state");
    let near = |d: (i32, i32, i32)| (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2);
    let same = |p| FluidCell::at(scene, p).is_some_and(|other| other.kind == cell.kind);
    let up = near((0, 1, 0));
    let down = near((0, -1, 0));
    let top = !same(up) && !scene.full_collision_at(up);
    let bottom = !same(down) && !scene.full_collision_at(down);
    // The side faces, north, south, west and east: most fluid lies inside
    // a body of it and shows none of its faces.
    let sides = [(0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)].map(|d| {
        let other = near(d);
        !(same(other) || scene.full_collision_at(other))
    });
    if !top && !bottom && !sides.contains(&true) && !FLUID_FULL_PATH.with(std::cell::Cell::get) {
        return Ok(());
    }
    let water = cell.kind == FluidKind::Water;
    static TEXTURES: std::sync::OnceLock<[ResourceId; 4]> = std::sync::OnceLock::new();
    let [water_still, water_flow, lava_still, lava_flow] = TEXTURES.get_or_init(|| {
        ["water_still", "water_flow", "lava_still", "lava_flow"].map(|name| ResourceId::parse(&format!("minecraft:block/{name}")).expect("static texture id"))
    });
    let (still, flow) = if water { (water_still.clone(), water_flow.clone()) } else { (lava_still.clone(), lava_flow.clone()) };
    let tint = if water {
        scene.biome_at(pos).water_color.map(|c| c as f32 / 255.0)
    } else {
        [1.0; 3]
    };
    let _ = tint_source;
    let alpha = if water { 0.72 } else { 1.0 };
    let [x, y, z] = [pos.0 as f32, pos.1 as f32, pos.2 as f32];
    let h = fluid_render_height(scene, cell.kind, pos);
    let nw;
    let ne;
    let sw;
    let se;
    if h >= 1.0 {
        nw = 1.0;
        ne = 1.0;
        sw = 1.0;
        se = 1.0;
    } else {
        let north = fluid_render_height(scene, cell.kind, near((0, 0, -1)));
        let south = fluid_render_height(scene, cell.kind, near((0, 0, 1)));
        let west = fluid_render_height(scene, cell.kind, near((-1, 0, 0)));
        let east = fluid_render_height(scene, cell.kind, near((1, 0, 0)));
        nw = fluid_corner_height(scene, cell.kind, h, north, west, near((-1, 0, -1)));
        ne = fluid_corner_height(scene, cell.kind, h, north, east, near((1, 0, -1)));
        sw = fluid_corner_height(scene, cell.kind, h, south, west, near((-1, 0, 1)));
        se = fluid_corner_height(scene, cell.kind, h, south, east, near((1, 0, 1)));
    }
    let sky = light.get(pos).max(light.get(up)) as f32;
    let color = |shade: f32| [shade * tint[0], shade * tint[1], shade * tint[2], alpha];
    let uv = |texture: &ResourceId, points: [[f32; 2]; 4]| {
        let [u0, v0, u1, v1] = atlas.region(texture);
        points.map(|[u, v]| [u0 + (u1 - u0) * u, v0 + (v1 - v0) * v])
    };
    if top {
        let mut dx = 0.0_f32;
        let mut dz = 0.0_f32;
        for d in [(0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)] {
            let target = near(d);
            let hh = fluid_render_height(scene, cell.kind, target);
            let delta = if hh > 0.0 {
                h - hh
            } else if !scene.full_collision_at(target) {
                let below = near((d.0, -1, d.2));
                let low = fluid_render_height(scene, cell.kind, below);
                if low > 0.0 {
                    h - (low - 8.0 / 9.0)
                } else {
                    0.0
                }
            } else {
                0.0
            };
            dx += d.0 as f32 * delta;
            dz += d.2 as f32 * delta;
        }
        let (texture, coords) = if dx == 0.0 && dz == 0.0 {
            (&still, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]])
        } else {
            let angle = dz.atan2(dx) - std::f32::consts::FRAC_PI_2;
            let (s, c) = (angle.sin() * 0.25, angle.cos() * 0.25);
            (
                &flow,
                [
                    [0.5 - c - s, 0.5 - c + s],
                    [0.5 - c + s, 0.5 + c + s],
                    [0.5 + c + s, 0.5 + c - s],
                    [0.5 + c - s, 0.5 - c - s],
                ],
            )
        };
        let texture = if atlas.contains(texture) {
            texture
        } else {
            &still
        };
        fluid_quad(
            mesh,
            transparent,
            [
                [x, y + nw - 0.001, z],
                [x, y + sw - 0.001, z + 1.0],
                [x + 1.0, y + se - 0.001, z + 1.0],
                [x + 1.0, y + ne - 0.001, z],
            ],
            uv(texture, coords),
            color(1.0),
            sky,
            light.get_block(pos) as f32,
            water,
        );
    }
    if bottom {
        fluid_quad(
            mesh,
            transparent,
            [
                [x, y + 0.001, z],
                [x + 1.0, y + 0.001, z],
                [x + 1.0, y + 0.001, z + 1.0],
                [x, y + 0.001, z + 1.0],
            ],
            uv(&still, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
            color(0.5),
            light.get(down) as f32,
            light.get_block(down) as f32,
            water,
        );
    }
    for (side, (_, points, heights, shade)) in [
        (
            (0, 0, -1),
            [[x, y, z + 0.001], [x + 1.0, y, z + 0.001]],
            [nw, ne],
            0.8,
        ),
        (
            (0, 0, 1),
            [[x + 1.0, y, z + 0.999], [x, y, z + 0.999]],
            [se, sw],
            0.8,
        ),
        (
            (-1, 0, 0),
            [[x + 0.001, y, z + 1.0], [x + 0.001, y, z]],
            [sw, nw],
            0.6,
        ),
        (
            (1, 0, 0),
            [[x + 0.999, y, z], [x + 0.999, y, z + 1.0]],
            [ne, se],
            0.6,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if !sides[side] {
            continue;
        }
        let [[ax, _, az], [bx, _, bz]] = points;
        let [ha, hb] = heights;
        fluid_quad(
            mesh,
            transparent,
            [
                [ax, y + ha - 0.001, az],
                [bx, y + hb - 0.001, bz],
                [bx, y, bz],
                [ax, y, az],
            ],
            uv(
                &flow,
                [
                    [0.0, (1.0 - ha) * 0.5],
                    [0.5, (1.0 - hb) * 0.5],
                    [0.5, 0.5],
                    [0.0, 0.5],
                ],
            ),
            color(shade),
            sky,
            light.get_block(pos) as f32,
            water,
        );
    }
    Ok(())
}
/// Draw only a newly placed block while its neighboring chunks are rebuilt on
/// the worker. This uses the same model, atlas, tint, AO and face rules as the
/// chunk path, but samples the already available light map for this brief pose.
pub fn block_preview<S: Scene>(
    scene: &S,
    packs: &PackStack,
    atlas: &Atlas,
    light: &SkyLight,
    tint_source: &BiomeTint,
    pos: BlockPos,
) -> Result<ChunkMesh> {
    let Some(block) = scene.block(pos) else {
        return Ok(ChunkMesh::default());
    };
    if matches!(block.id.path.as_str(), "water" | "lava")
        || block
            .properties
            .get("waterlogged")
            .is_some_and(|value| value == "true")
    {
        let mut mesh = ChunkMesh::default();
        let mut transparent = Vec::new();
        append_fluid(
            scene,
            pos,
            atlas,
            tint_source,
            light,
            &mut mesh,
            &mut transparent,
        )?;
        if !transparent.is_empty() {
            mesh.transparent_start = Some(mesh.indices.len() as u32);
            mesh.indices.extend(transparent);
        }
        return Ok(mesh);
    }
    let variants = resolve_block_variants(packs, block)?;
    let model = &variants[variant_index(&variants, pos)].0;
    let biome = scene.biome_at(pos);
    let mut mesh = ChunkMesh::default();
    let mut transparent_indices = Vec::new();
    let mut neighbor_cube = HashMap::new();
    for element in &model.elements {
        for face in &element.faces {
            if !atlas.contains(&face.texture) {
                return Err(anyhow!("atlas missing {}", face.texture.key()));
            }
            let dir = rotated_direction(face.direction.as_str(), element.rotation_y)?;
            let delta = direction(dir)?;
            let neighbor_pos = (pos.0 + delta.0, pos.1 + delta.1, pos.2 + delta.2);
            if face.cull {
                if let Some(neighbor) = scene.block(neighbor_pos) {
                    let full_cube = if neighbor.is_opaque() {
                        let key = model_key(neighbor);
                        if let Some(&cube) = neighbor_cube.get(&key) {
                            cube
                        } else {
                            let cube = resolve_block_variants(packs, neighbor)?.iter().all(
                                |(model, _)| {
                                    model
                                        .elements
                                        .iter()
                                        .any(|part| part.from == [0.0; 3] && part.to == [1.0; 3])
                                },
                            );
                            neighbor_cube.insert(key, cube);
                            cube
                        }
                    } else {
                        false
                    };
                    if hides_shared_face(block, neighbor) || (neighbor.is_opaque() && full_cube) {
                        continue;
                    }
                }
            }
            let corners = element_corners(element, &face.direction)?;
            let [u0, v0, u1, v1] = if block.id.path == "chest" {
                // Chest ModelPart UVs address individual pixels in a 64x64
                // entity sheet. The terrain atlas' half-texel tile inset
                // distorts one-pixel lock faces and can sample alpha beside
                // them, so preserve the entity sheet's exact coordinate span.
                atlas.region_exact(&face.texture)
            } else {
                atlas.region(&face.texture)
            };
            let [a, b, c, d] = face.uv;
            let uv = [
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * b],
                [u0 + (u1 - u0) * a, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * d],
                [u0 + (u1 - u0) * c, v0 + (v1 - v0) * b],
            ];
            let shade = if block.id.path == "chest" {
                level_item_shade(face_normal(dir)?)
            } else {
                match element.shade_direction_override.as_deref().unwrap_or(dir) {
                    "up" => 1.0,
                    "down" => 0.5,
                    "north" | "south" => 0.8,
                    _ => 0.6,
                }
            };
            let tint = block_face_tint(block, face.tint, biome, tint_source);
            let alpha = if block.id.path == "water" { 0.72 } else { 1.0 };
            // ChestRenderer lights every ModelPart face from the block entity
            // position; a rotated lid must not retain its closed downward
            // face's dark floor-neighbor light.
            let light_pos = if block.id.path == "chest" {
                pos
            } else {
                neighbor_pos
            };
            let full_cube = element.from == [0.0; 3] && element.to == [1.0; 3];
            let start = mesh.vertices.len() as u32;
            for (corner, uv) in corners.iter().zip(uv) {
                let (ao, sky_light) = if full_cube {
                    ambient_vertex(scene, light, light_pos, dir, *corner)
                } else {
                    (1.0, light.get(light_pos) as f32)
                };
                mesh.vertices.push(Vertex {
                    position: [
                        pos.0 as f32 + corner[0],
                        pos.1 as f32 + corner[1],
                        pos.2 as f32 + corner[2],
                    ],
                    uv,
                    color: [
                        shade * ao * tint[0],
                        shade * ao * tint[1],
                        shade * ao * tint[2],
                        alpha,
                    ],
                    sky_light,
                    block_light: light.get_block(light_pos) as f32,
                });
            }
            let indices = if block.id.path == "water" || face.force_translucent {
                &mut transparent_indices
            } else {
                &mut mesh.indices
            };
            indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
            mesh.faces += 1;
        }
    }
    if !transparent_indices.is_empty() {
        mesh.transparent_start = Some(mesh.indices.len() as u32);
        mesh.indices.extend(transparent_indices);
    }
    Ok(mesh)
}
/// HalfTransparentBlock skips its own neighboring face. Fancy cutout leaves
/// retain shared faces in 26.3; rasterizer back-face culling hides their backs.
pub(crate) fn hides_shared_face(block: &Block, neighbor: &Block) -> bool {
    if block.id != neighbor.id {
        return false;
    }
    let path = block.id.path.as_str();
    path == "water" || path == "glass" || path == "tinted_glass" || path.ends_with("_stained_glass")
}

/// Full-cube case of BlockModelLighter's four neighboring shade and light
/// samples. Keeping values per vertex produces the smooth block-edge fade.
fn ambient_vertex<S: Scene>(
    scene: &S,
    light: &SkyLight,
    base: (i32, i32, i32),
    dir: &str,
    corner: [f32; 3],
) -> (f32, f32) {
    let axes = match dir {
        "up" | "down" => [0, 2],
        "north" | "south" => [0, 1],
        _ => [1, 2],
    };
    let mut side = [[0; 3]; 2];
    for (i, axis) in axes.into_iter().enumerate() {
        side[i][axis] = if corner[axis] < 0.5 { -1 } else { 1 };
    }
    let positions = [
        base,
        (
            base.0 + side[0][0],
            base.1 + side[0][1],
            base.2 + side[0][2],
        ),
        (
            base.0 + side[1][0],
            base.1 + side[1][1],
            base.2 + side[1][2],
        ),
        (
            base.0 + side[0][0] + side[1][0],
            base.1 + side[0][1] + side[1][1],
            base.2 + side[0][2] + side[1][2],
        ),
    ];
    let center_light = light.get(base);
    let shade = positions
        .iter()
        .map(|&p| if scene.shade_darkens_at(p) { 0.2 } else { 1.0 })
        .sum::<f32>()
        * 0.25;
    let sky = positions
        .iter()
        .map(|&p| {
            let level = light.get(p);
            if center_light > 2 && level == 0 {
                center_light as f32
            } else {
                level as f32
            }
        })
        .sum::<f32>()
        * 0.25;
    (shade, sky)
}
fn direction(dir: &str) -> Result<(i32, i32, i32)> {
    Ok(match dir {
        "down" => (0, -1, 0),
        "up" => (0, 1, 0),
        "north" => (0, 0, -1),
        "south" => (0, 0, 1),
        "west" => (-1, 0, 0),
        "east" => (1, 0, 0),
        _ => return Err(anyhow!("unsupported face direction {dir}")),
    })
}
fn corners(dir: &str, a: [f32; 3], b: [f32; 3]) -> Result<[[f32; 3]; 4]> {
    let [x, y, z] = a;
    let [xx, yy, zz] = b;
    Ok(match dir {
        "up" => [[x, yy, z], [x, yy, zz], [xx, yy, zz], [xx, yy, z]],
        "down" => [[x, y, zz], [x, y, z], [xx, y, z], [xx, y, zz]],
        "north" => [[xx, yy, z], [xx, y, z], [x, y, z], [x, yy, z]],
        "south" => [[x, yy, zz], [x, y, zz], [xx, y, zz], [xx, yy, zz]],
        "west" => [[x, yy, z], [x, y, z], [x, y, zz], [x, yy, zz]],
        "east" => [[xx, yy, zz], [xx, y, zz], [xx, y, z], [xx, yy, z]],
        _ => return Err(anyhow!("unsupported face direction {dir}")),
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The `cross` model's planes: rotated 45° about the block's middle and
    /// rescaled, they run corner to corner; a plane in the middle of the
    /// block takes its light from its own cell, a face on the block's side
    /// from the neighbour (`BlockModelLighter.faceCubic`).
    #[test]
    fn cross_planes_turn_diagonal_and_light_from_their_own_cell() {
        let raw: serde_json::Value = serde_json::from_str(r#"{"rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true}}"#).unwrap();
        let element = crate::model::Element {
            from: [0.8 / 16.0, 0.0, 0.5],
            to: [15.2 / 16.0, 1.0, 0.5],
            faces: Vec::new(),
            rotation_y: 0,
            rotation: crate::model::ElementRotation::parse(&raw).unwrap(),
            shade_direction_override: None,
        };
        let corners = element_corners(&element, "north").unwrap();
        for (corner, (x, z)) in corners.iter().zip([(0.95, 0.05), (0.95, 0.05), (0.05, 0.95), (0.05, 0.95)]) {
            assert!((corner[0] - x).abs() < 1e-5 && (corner[2] - z).abs() < 1e-5, "{corner:?}");
        }
        // The normal points between north and west; north comes first.
        assert_eq!(quad_direction(&element, "north", &corners).unwrap(), "north");
        assert_eq!(quad_direction(&element, "south", &element_corners(&element, "south").unwrap()).unwrap(), "south");
        assert!(!face_cubic("north", &corners), "a diagonal plane is lit from its own cell");
        let cube = crate::model::Element { from: [0.0; 3], to: [1.0; 3], faces: Vec::new(), rotation_y: 0, rotation: None, shade_direction_override: None };
        let top = element_corners(&cube, "up").unwrap();
        assert!(face_cubic("up", &top), "a cube's top lies on its side");
        let slab = crate::model::Element { from: [0.0; 3], to: [1.0, 0.5, 1.0], faces: Vec::new(), rotation_y: 0, rotation: None, shade_direction_override: None };
        assert!(!face_cubic("up", &element_corners(&slab, "up").unwrap()), "a slab's top is inside the block");
        // A 22.5° torch tilt about z: no rescale.
        let raw: serde_json::Value = serde_json::from_str(r#"{"rotation":{"origin":[0,3.5,8],"axis":"z","angle":-22.5}}"#).unwrap();
        let tilt = crate::model::ElementRotation::parse(&raw).unwrap().unwrap();
        let [x, y, _] = tilt.apply([0.0, 3.5 / 16.0 + 1.0, 0.5]);
        assert!((x - (22.5f32).to_radians().sin()).abs() < 1e-5 && (y - (3.5 / 16.0 + (22.5f32).to_radians().cos())).abs() < 1e-5, "{x} {y}");
    }
    use crate::scene::{BlockPos, ChunkPos, HandcraftedScene};

    #[test]
    fn entity_sheet_region_restores_exact_pixel_coordinates() {
        let texture = ResourceId::parse("minecraft:entity/chest/normal_left").unwrap();
        let atlas = Atlas {
            pixels: RgbaImage::new(64, 64),
            mipmaps: vec![],
            slots: HashMap::from([(
                texture.clone(),
                [0.5 / 64.0, 0.5 / 64.0, 63.5 / 64.0, 63.5 / 64.0],
            )]),
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        assert_eq!(atlas.region_exact(&texture), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn entity_skin_uvs_keep_internal_pixel_boundaries_exact() {
        let id = ResourceId::parse("minecraft:entity/zombie/zombie").unwrap();
        let mut slots = HashMap::new();
        slots.insert(
            id.clone(),
            [64.5 / 128.0, 0.5 / 128.0, 127.5 / 128.0, 63.5 / 128.0],
        );
        let atlas = Atlas {
            pixels: RgbaImage::new(128, 128),
            mipmaps: Vec::new(),
            slots,
            missing: Vec::new(),
            animated: Vec::new(),
            animated_tiles: Vec::new(),
        };
        let [u0, v0, u1, v1] = atlas.entity_region(&id);
        assert_eq!([u0, v0, u1, v1], [0.5, 0.0, 1.0, 0.5]);
        // The zombie hand starts at skin pixel 48. With the regular inset,
        // that boundary lands at 47.75 and selects a shirt pixel instead.
        assert_eq!((u0 + (u1 - u0) * 48.0 / 64.0) * 128.0, 112.0);
        assert!(
            (atlas.region(&id)[0] + (atlas.region(&id)[2] - atlas.region(&id)[0]) * 48.0 / 64.0)
                * 128.0
                < 112.0
        );
    }

    #[test]
    fn wire_tint_uses_power_and_leaves_overlay_untinted() {
        let source = BiomeTint {
            grass: None,
            foliage: None,
        };
        let wire = Block::new("minecraft:redstone_wire").with("power", "15");
        let tinted = block_face_tint(&wire, true, BiomeSample::THE_VOID, &source);
        assert_eq!(redstone_wire_color_argb(0), 0xff4c0000);
        assert_eq!(redstone_wire_color_argb(15), 0xffff3200);
        assert_eq!(tinted, [1.0, 50.0 / 255.0, 0.0]);
        assert_eq!(
            block_face_tint(&wire, false, BiomeSample::THE_VOID, &source),
            [1.0; 3]
        );
    }

    #[test]
    fn discrete_frames_follow_pack_order_and_tick_durations() {
        let mut sheet = RgbaImage::new(2, 6);
        for frame in 0..3 {
            for y in frame * 2..frame * 2 + 2 {
                for x in 0..2 {
                    sheet.put_pixel(x, y, image::Rgba([frame as u8 * 40, 0, 0, 255]));
                }
            }
        }
        let metadata = serde_json::json!({
            "frametime": 2,
            "frames": [0, {"index": 2, "time": 3}, 1]
        });
        let tile = animated_tile(&sheet, &metadata, (64, 128)).unwrap();
        assert_eq!(tile.origin, (64, 128));
        assert_eq!(
            (0..7).map(|tick| tile.frame_at(tick)).collect::<Vec<_>>(),
            [0, 0, 2, 2, 2, 1, 1]
        );
        assert_eq!(tile.frame_at(7), 0);
        assert_eq!(tile.frames[2][0].get_pixel(0, 0).0, [80, 0, 0, 255]);
        assert_eq!(tile.frames[2][4].dimensions(), (4, 4));
    }

    #[test]
    fn interpolated_sprite_uses_pinned_thousandth_progress() {
        let mut sheet = RgbaImage::new(2, 4);
        for y in 2..4 {
            for x in 0..2 {
                sheet.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let tile = animated_tile(
            &sheet,
            &serde_json::json!({"frametime":8,"interpolate":true}),
            (0, 0),
        )
        .unwrap();
        let state = tile.state_at(4);
        assert_eq!(state.progress_millis, 500);
        assert_eq!((state.current, state.next), (0, 1));
        assert_eq!(tile.image_at(state, 0).get_pixel(0, 0).0, [128, 0, 0, 128]);
    }
    #[test]
    fn dropped_item_shadow_projects_on_ground() {
        let shadow = ResourceId::parse("minecraft:misc/shadow").unwrap();
        let atlas = Atlas {
            pixels: RgbaImage::new(32, 32),
            mipmaps: vec![],
            slots: HashMap::from([(shadow, [0.0, 0.0, 1.0, 1.0])]),
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        let scene = HandcraftedScene::new();
        let mut items = WorldItems::default();
        items.spawn(
            minecraftoss_player::inventory::ItemStack::new("minecraft:stone", 1),
            glam::DVec3::new(0.5, 1.0, 0.5),
        );
        let casters = item_shadow_casters(&items, glam::DVec3::new(0.5, 2.62, 2.5), 1.0);
        let shadow_mesh = entity_shadows(&casters, &scene, &atlas, &|_| 15, 0.0).unwrap();
        assert!(shadow_mesh.faces >= 1);
        assert!(shadow_mesh
            .vertices
            .iter()
            .any(|vertex| vertex.position[1] > 1.0));
        assert!(shadow_mesh
            .vertices
            .iter()
            .all(|vertex| vertex.color[3] > 0.0));
    }
    #[test]
    fn dropped_grass_uses_its_item_tint_face_lighting_and_local_skylight() {
        let texture = ResourceId::parse("minecraft:block/grass_block_top").unwrap();
        let atlas = Atlas {
            pixels: RgbaImage::new(16, 16),
            mipmaps: vec![],
            slots: HashMap::from([(texture.clone(), [0.0, 0.0, 1.0, 1.0])]),
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        let face = |direction: &str, tint_index: Option<usize>| crate::model::Face {
            direction: direction.into(),
            texture: texture.clone(),
            uv: [0.0, 0.0, 1.0, 1.0],
            cull: false,
            cullface: None,
            tint: tint_index.is_some(),
            tint_index,
            force_translucent: false,
        };
        let mut visuals = ItemVisuals::default();
        visuals.models.insert(
            "minecraft:grass_block".into(),
            ItemVisual {
                model: ResolvedModel {
                    elements: vec![crate::model::Element {
                        from: [0.0; 3],
                        to: [1.0; 3],
                        faces: vec![face("up", Some(0)), face("north", None)],
                        rotation_y: 0,
                        rotation: None,
                        shade_direction_override: None,
                    }],
                },
                ground: Mat4::IDENTITY,
                right_hand: Mat4::IDENTITY,
                first_person: Mat4::IDENTITY,
                min_y: 0.0,
                tints: vec![[0.3, 0.6, 0.2]],
                flat: false,
            },
        );
        let mut items = WorldItems::default();
        items.spawn(
            minecraftoss_player::inventory::ItemStack::new("minecraft:grass_block", 1),
            glam::DVec3::new(5.5, 6.0, -6.5),
        );
        let sky_light = SkyLight::build(&HandcraftedScene::new());
        let mesh = visuals
            .mesh(
                &items,
                &PackStack::open(vec![]).unwrap(),
                &atlas,
                &sky_light,
                1.0,
            )
            .unwrap();
        assert_eq!(mesh.vertices.len(), 8);
        for vertex in &mesh.vertices[..4] {
            for (actual, expected) in vertex.color[..3].iter().zip([0.3, 0.6, 0.2]) {
                assert!((actual - expected).abs() < 0.0001);
            }
        }
        assert!(mesh.vertices[4].color[0] < 0.95);
        assert!(mesh.vertices.iter().all(|vertex| vertex.sky_light == 13.0));
    }
    #[test]
    #[ignore = "local performance diagnostic with the pinned resource pack"]
    fn local_block_edit_mesh_timing() {
        let Ok(path) = std::env::var("MINECRAFTOSS_PACK") else {
            return;
        };
        if !std::path::Path::new(&path).exists() {
            return;
        }
        let packs = PackStack::open(vec![path.into()]).unwrap();
        let mut scene = HandcraftedScene::new();
        let build = build(&scene, &packs).unwrap();
        scene.set((0, 2, 3), Some(Block::new("minecraft:glass")));
        let tint = BiomeTint::from_pack(&packs).unwrap();
        let mut preview_times = Vec::new();
        let mut times = Vec::new();
        let mut light_times = Vec::new();
        let mut clone_times = Vec::new();
        for _ in 0..5 {
            let preview_start = std::time::Instant::now();
            let preview = block_preview(
                &scene,
                &packs,
                &build.atlas,
                build.sky_light.as_deref().unwrap(),
                &tint,
                (0, 2, 3),
            )
            .unwrap();
            preview_times.push(preview_start.elapsed().as_secs_f64() * 1000.0);
            assert!(preview.faces > 0);
            let clone_start = std::time::Instant::now();
            let _ = scene.clone();
            clone_times.push(clone_start.elapsed().as_secs_f64() * 1000.0);
            let light_start = std::time::Instant::now();
            let _ = SkyLight::build(&scene);
            light_times.push(light_start.elapsed().as_secs_f64() * 1000.0);
            let start = std::time::Instant::now();
            let changed = rebuild_near(&scene, &packs, &build, (0, 2, 3)).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert!(!changed.is_empty());
        }
        eprintln!("nearby chunk rebuild ms: {times:?}");
        eprintln!("single block preview ms: {preview_times:?}");
        eprintln!("skylight rebuild ms: {light_times:?}");
        eprintln!("scene dispatch clone ms: {clone_times:?}");
        let near = rebuild_near(&scene, &packs, &build, (0, 2, 3)).unwrap();
        let full = super::build(&scene, &packs).unwrap();
        for (chunk, mesh) in near {
            let canonical = full.chunks.get(&chunk).unwrap();
            assert_eq!(mesh.indices, canonical.indices);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&mesh.vertices),
                bytemuck::cast_slice::<_, u8>(&canonical.vertices)
            );
        }
    }
    struct Two {
        a: Block,
        b: Block,
    }
    impl Scene for Two {
        fn block(&self, p: BlockPos) -> Option<&Block> {
            match p {
                (15, 0, 0) => Some(&self.a),
                (16, 0, 0) => Some(&self.b),
                _ => None,
            }
        }
        fn chunks(&self) -> Vec<ChunkPos> {
            vec![(0, 0), (1, 0)]
        }
        fn vertical_range(&self) -> std::ops::Range<i32> {
            0..1
        }
        fn revision(&self) -> u64 {
            0
        }
    }
    fn test_box_model(texture: &ResourceId, from: [f32; 3], to: [f32; 3]) -> ResolvedModel {
        ResolvedModel {
            elements: vec![crate::model::Element {
                from,
                to,
                rotation_y: 0,
                rotation: None,
                shade_direction_override: None,
                faces: ["down", "up", "north", "south", "west", "east"]
                    .map(|direction| crate::model::Face {
                        direction: direction.into(),
                        texture: texture.clone(),
                        uv: [0.0, 0.0, 1.0, 1.0],
                        cull: true,
                        cullface: None,
                        tint: false,
                        tint_index: None,
                        force_translucent: false,
                    })
                    .to_vec(),
            }],
        }
    }

    #[test]
    fn partial_neighbors_keep_the_full_block_face() {
        let texture = ResourceId::parse("minecraft:block/stone").unwrap();
        let atlas = Atlas {
            pixels: RgbaImage::new(16, 16),
            mipmaps: vec![],
            slots: HashMap::from([(texture.clone(), [0.0, 0.0, 1.0, 1.0])]),
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        let tint = BiomeTint {
            grass: None,
            foliage: None,
        };
        for (neighbor_id, neighbor_model, expected_faces) in [
            (
                "minecraft:oak_slab",
                test_box_model(&texture, [0.0; 3], [1.0, 0.5, 1.0]),
                6,
            ),
            (
                "minecraft:oak_stairs",
                test_box_model(&texture, [0.0; 3], [1.0, 0.5, 1.0]),
                6,
            ),
            (
                "minecraft:oak_slab",
                test_box_model(&texture, [0.0; 3], [1.0; 3]),
                5,
            ),
        ] {
            let scene = Two {
                a: Block::new("minecraft:stone"),
                b: Block::new(neighbor_id),
            };
            let models = HashMap::from([
                (
                    model_key(&scene.a),
                    vec![(test_box_model(&texture, [0.0; 3], [1.0; 3]), 1)],
                ),
                (model_key(&scene.b), vec![(neighbor_model, 1)]),
            ]);
            let light = SkyLight::build(&scene);
            assert_eq!(
                mesh_chunk(&scene, (0, 0), &models, &atlas, &tint, &light)
                    .unwrap()
                    .faces,
                expected_faces,
                "neighbor {neighbor_id}"
            );
        }
    }

    #[test]
    fn water_faces_use_the_translucent_range_and_hide_shared_faces() {
        let texture = ResourceId::parse("minecraft:block/water_still").unwrap();
        let atlas = Atlas {
            pixels: RgbaImage::new(16, 16),
            mipmaps: vec![],
            slots: HashMap::from([(texture.clone(), [0.0, 0.0, 1.0, 1.0])]),
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        let tint = BiomeTint {
            grass: None,
            foliage: None,
        };
        let water_model = test_box_model(&texture, [0.0; 3], [1.0, 8.0 / 9.0, 1.0]);
        let solid_model = test_box_model(&texture, [0.0; 3], [1.0; 3]);
        for neighbor in ["minecraft:water", "minecraft:stone"] {
            let scene = Two {
                a: Block::new("minecraft:water"),
                b: Block::new(neighbor),
            };
            let models = HashMap::from([
                (model_key(&scene.a), vec![(water_model.clone(), 1)]),
                (
                    model_key(&scene.b),
                    vec![(
                        if neighbor == "minecraft:water" {
                            water_model.clone()
                        } else {
                            solid_model.clone()
                        },
                        1,
                    )],
                ),
            ]);
            let light = SkyLight::build(&scene);
            let water = mesh_chunk(&scene, (0, 0), &models, &atlas, &tint, &light).unwrap();
            assert_eq!(water.faces, 5);
            assert_eq!(water.transparent_start, Some(0));
            assert_eq!(water.indices.len(), water.faces * 6);
            if neighbor == "minecraft:stone" {
                let solid = mesh_chunk(&scene, (1, 0), &models, &atlas, &tint, &light).unwrap();
                assert_eq!(solid.faces, 6);
                assert_eq!(solid.transparent_start, None);
            }
        }
    }
    #[test]
    fn forced_translucent_model_face_uses_blended_range() {
        let texture = ResourceId::parse("minecraft:block/redstone_dust_dot").unwrap();
        let atlas = Atlas {
            pixels: RgbaImage::new(16, 16),
            mipmaps: vec![],
            slots: HashMap::from([(texture.clone(), [0.0, 0.0, 1.0, 1.0])]),
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        let mut model = test_box_model(&texture, [0.0; 3], [1.0; 3]);
        model.elements[0].faces[0].force_translucent = true;
        let mut scene = HandcraftedScene::default();
        scene.set((0, 0, 0), Some(Block::new("minecraft:redstone_wire")));
        let block = scene.block((0, 0, 0)).unwrap();
        let models = HashMap::from([(model_key(block), vec![(model, 1)])]);
        let mesh = mesh_chunk(
            &scene,
            (0, 0),
            &models,
            &atlas,
            &BiomeTint {
                grass: None,
                foliage: None,
            },
            &SkyLight::build(&scene),
        )
        .unwrap();
        assert_eq!(mesh.transparent_start, Some(30));
        assert_eq!(mesh.indices.len(), 36);
    }
    #[test]
    fn flowing_surface_height_and_lava_light_follow_fluid_state() {
        let mut scene = HandcraftedScene::default();
        scene.set(
            (0, 0, 0),
            Some(Block::new("minecraft:water").with("level", "4")),
        );
        scene.set(
            (2, 0, 0),
            Some(Block::new("minecraft:lava").with("level", "0")),
        );
        let ids = ["water_still", "water_flow", "lava_still", "lava_flow"];
        let slots = ids
            .into_iter()
            .map(|name| {
                (
                    ResourceId::parse(&format!("minecraft:block/{name}")).unwrap(),
                    [0.0, 0.0, 1.0, 1.0],
                )
            })
            .collect();
        let atlas = Atlas {
            pixels: RgbaImage::new(16, 16),
            mipmaps: vec![],
            slots,
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        let tint = BiomeTint {
            grass: None,
            foliage: None,
        };
        let light = SkyLight::build(&scene);
        let mut water = ChunkMesh::default();
        let mut transparent = Vec::new();
        append_fluid(
            &scene,
            (0, 0, 0),
            &atlas,
            &tint,
            &light,
            &mut water,
            &mut transparent,
        )
        .unwrap();
        assert!(!transparent.is_empty());
        assert!(water.vertices.iter().all(|v| v.position[1] <= 4.0 / 9.0));
        let mut lava = ChunkMesh::default();
        let mut transparent = Vec::new();
        append_fluid(
            &scene,
            (2, 0, 0),
            &atlas,
            &tint,
            &light,
            &mut lava,
            &mut transparent,
        )
        .unwrap();
        assert!(transparent.is_empty());
        assert!(lava.vertices.iter().all(|v| v.block_light >= 14.0));
        assert!(lava.vertices.iter().any(|v| v.block_light == 15.0));
    }
    #[test]
    fn boundary_neighbors_hide_shared_face() {
        let id = ResourceId::parse("minecraft:block/stone").unwrap();
        let atlas = Atlas {
            pixels: RgbaImage::new(32, 32),
            mipmaps: vec![],
            slots: HashMap::from([(id.clone(), [0.0, 0.0, 1.0, 1.0])]),
            missing: vec![],
            animated: vec![],
            animated_tiles: vec![],
        };
        let model = ResolvedModel {
            elements: vec![crate::model::Element {
                from: [0.0; 3],
                to: [1.0; 3],
                rotation_y: 0,
                rotation: None,
                shade_direction_override: None,
                faces: ["down", "up", "north", "south", "west", "east"]
                    .map(|d| crate::model::Face {
                        direction: d.into(),
                        texture: id.clone(),
                        uv: [0.0, 0.0, 1.0, 1.0],
                        cull: true,
                        cullface: None,
                        tint: false,
                        tint_index: None,
                        force_translucent: false,
                    })
                    .to_vec(),
            }],
        };
        let scene = Two {
            a: Block::new("minecraft:stone"),
            b: Block::new("minecraft:stone"),
        };
        let models = HashMap::from([(model_key(&scene.a), vec![(model, 1)])]);
        let light = SkyLight::build(&scene);
        assert_eq!(
            mesh_chunk(
                &scene,
                (0, 0),
                &models,
                &atlas,
                &BiomeTint {
                    grass: None,
                    foliage: None
                },
                &light
            )
            .unwrap()
            .faces,
            5
        );
        assert_eq!(
            mesh_chunk(
                &scene,
                (1, 0),
                &models,
                &atlas,
                &BiomeTint {
                    grass: None,
                    foliage: None
                },
                &light
            )
            .unwrap()
            .faces,
            5
        );
    }

    #[test]
    fn glass_hides_shared_faces_while_fancy_leaves_keep_them() {
        let glass = Block::new("minecraft:glass");
        let leaves = Block::new("minecraft:oak_leaves");
        assert!(hides_shared_face(&glass, &glass));
        assert!(!hides_shared_face(&leaves, &leaves));
        assert!(!hides_shared_face(
            &glass,
            &Block::new("minecraft:tinted_glass")
        ));
    }

    #[test]
    fn face_corners_blend_neighbor_shade() {
        let mut scene = HandcraftedScene::new();
        scene.set((-1, 1, -1), Some(Block::new("minecraft:stone")));
        let light = SkyLight::build(&scene);
        let shaded = ambient_vertex(&scene, &light, (0, 1, 0), "up", [0.0, 1.0, 0.0]);
        let open = ambient_vertex(&scene, &light, (0, 1, 0), "up", [1.0, 1.0, 1.0]);
        assert!(shaded.0 < open.0);
    }

    #[test]
    fn cube_vertices_follow_minecraft_faceinfo_order() {
        assert_eq!(
            corners("up", [0.0; 3], [1.0; 3]).unwrap()[0],
            [0.0, 1.0, 0.0]
        );
        assert_eq!(
            corners("down", [0.0; 3], [1.0; 3]).unwrap()[0],
            [0.0, 0.0, 1.0]
        );
        assert_eq!(
            corners("north", [0.0; 3], [1.0; 3]).unwrap()[0],
            [1.0, 1.0, 0.0]
        );
        assert_eq!(
            corners("south", [0.0; 3], [1.0; 3]).unwrap()[0],
            [0.0, 1.0, 1.0]
        );
        assert_eq!(
            corners("west", [0.0; 3], [1.0; 3]).unwrap()[0],
            [0.0, 1.0, 0.0]
        );
        assert_eq!(
            corners("east", [0.0; 3], [1.0; 3]).unwrap()[0],
            [1.0, 1.0, 1.0]
        );
    }

    #[test]
    fn position_seed_selects_grass_rotations() {
        let variants = (0..4)
            .map(|_| (ResolvedModel { elements: vec![] }, 1))
            .collect::<Vec<_>>();
        assert_eq!(variant_index(&variants, (0, 1, 0)), 2);
        assert_eq!(variant_index(&variants, (0, 1, 1)), 3);
        assert_eq!(variant_index(&variants, (-1, 1, 0)), 1);
        assert_eq!(variant_index(&variants, (2_000_000_000, 1, 0)), 1);
        assert_eq!(rotate_y([0.0, 1.0, 0.0], 90), [1.0, 1.0, 0.0]);
        assert_eq!(rotated_direction("north", 90).unwrap(), "east");
    }
}
