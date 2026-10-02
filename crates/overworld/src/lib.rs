//! MinecraftOSS world backend for the CS:GO simulation. Assets stay external.
pub mod adventure;
pub mod block_audio;
pub mod rendering;
mod dragon_model;
mod mobs;
pub mod survival;
use collision::{DynamicWorld, Trace, contents};
use glam::Vec3;
pub use glam_mc;
pub use minecraft_terrain as terrain;
use minecraft_terrain::{
    mesh::Atlas,
    pack::PackStack,
    scene::{Block, BlockPos, HandcraftedScene, Scene},
    server::{PlayerEdit, ServerHandle, ServerSim},
    terrain::{Dimension, TerrainStream},
};
pub use minecraftoss_core as core;
use minecraftoss_core::registries::{DataPaths, Registries};
pub use minecraftoss_entities as mc_entities;
pub use minecraftoss_player as mc_player;
use minecraftoss_player::World as PlayerWorld;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

/// 55% of the standing CS player hull; two blocks leave 7.2 units of headroom.
pub const BLOCK: f32 = 72.0 * 0.55;
/// Chunk generation threads during play. Generation runs at background
/// priority but still competes with the main and render threads; bursts of
/// new chunks at each chunk border caused 25-35 ms frames with the default
/// pool (three quarters of the cores), so play keeps two (pre-generation
/// uses all of them). `MINECRAFTOSS_PLAY_GEN_THREADS` overrides.
pub fn play_generation_threads() -> usize {
    std::env::var("MINECRAFTOSS_PLAY_GEN_THREADS").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0).unwrap_or(2)
}
pub fn is_map(name: &str) -> bool {
    name.eq_ignore_ascii_case("overworld") || name.eq_ignore_ascii_case("minecraft:overworld")
}
pub fn asset_root() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    for key in ["CSR_MINECRAFT_DIR", "MINECRAFTOSS_ROOT"] {
        if let Some(p) = std::env::var_os(key) {
            candidates.push(PathBuf::from(p));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(home).join("Downloads/2010-Rust-Rewrite-Mashup-windows-x64/2010-Rust-Rewrite-Mashup/iw4l-artifacts/minecraft-26.3"));
    }
    candidates
        .into_iter()
        .find(|p| {
            p.join("artifacts/block-state-catalog/26.3.json").is_file()
                && p.join("resourcepacks/local/minecraft-26.3").is_dir()
        })
        .ok_or_else(|| {
            "Minecraft assets not found. Set CSR_MINECRAFT_DIR to the minecraft-26.3 folder.".into()
        })
}
/// Minecraft saves, in the player data folder (the launcher runs inside it).
pub fn save_root() -> PathBuf {
    std::env::var_os("CSR_WORLD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("worlds"))
}
pub fn to_block(origin: [f64; 3], p: Vec3) -> [f64; 3] {
    [
        origin[0] + p.x as f64 / BLOCK as f64,
        origin[1] + p.z as f64 / BLOCK as f64,
        origin[2] - p.y as f64 / BLOCK as f64,
    ]
}
pub fn to_source(origin: [f64; 3], p: [f64; 3]) -> Vec3 {
    Vec3::new(
        (p[0] - origin[0]) as f32,
        -(p[2] - origin[2]) as f32,
        (p[1] - origin[1]) as f32,
    ) * BLOCK
}
pub fn block_pos(p: [f64; 3]) -> BlockPos {
    (
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    )
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Save {
    pub version: u32,
    pub seed: i64,
    pub origin: [f64; 3],
    pub player: [f64; 3],
    pub day_ticks: f64,
    pub items: Vec<(String, u32)>,
    pub selected: usize,
    #[serde(default)]
    pub survival: survival::Survival,
    #[serde(default)]
    pub adventure: adventure::Adventure,
}
/// Accepted mob damage in Source coordinates, consumed once by the effects renderer.
#[derive(Clone, Copy, Debug)]
pub struct MobHitFx { pub point: Vec3, pub from: Vec3 }
pub enum Interaction {
    Shot {
        point: Vec3,
        normal: Vec3,
        damage: f32,
    },
    Mob {
        key: u64,
        damage: f32,
        from: Vec3,
        point: Vec3,
    },
}
pub struct Collision {
    pub scene: Arc<RwLock<HandcraftedScene>>,
    pub origin: [f64; 3],
    pub events: Mutex<Vec<Interaction>>,
    pub targets: RwLock<Vec<(u64, Vec3, Vec3)>>,
}
impl Collision {
    fn trace_context(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3, mask: i32, player:Option<bool>) -> Trace {
        let scene = self.scene.read().unwrap();
        let mut out = Trace::empty(end);
        if mask & contents::SOLID == 0 {
            return out;
        }
        let steps = ((end - start).length() / (BLOCK * 2.0)).ceil().max(1.0) as usize;
        for step in 0..steps.min(8192) {
            let a = start.lerp(end, step as f32 / steps as f32);
            let b = start.lerp(end, (step + 1) as f32 / steps as f32);
            let lo = to_block(self.origin, a.min(b) + mins - Vec3::splat(1.0));
            let hi = to_block(self.origin, a.max(b) + maxs + Vec3::splat(1.0));
            let l = block_pos([lo[0].min(hi[0]), lo[1].min(hi[1]), lo[2].min(hi[2])]);
            let h = block_pos([lo[0].max(hi[0]), lo[1].max(hi[1]), lo[2].max(hi[2])]);
            for x in l.0..=h.0 {
                for z in l.2..=h.2 {
                    for y in l.1 - 1..=h.1 {
                        // Unloaded columns form a boundary, never a hole to fall through.
                        let scaffold=Scene::block(&*scene,(x,y,z)).is_some_and(|b|b.id.path=="scaffolding");
                        let shapes = if scaffold && player.is_some() {
                            let top=to_source(self.origin,[x as f64,(y+1) as f64,z as f64]).z;
                            if player==Some(false) && end.z<=start.z && start.z+mins.z>=top-0.1 {
                                vec![[0.,14./16.,0.,1.,1.,1.]]
                            } else {vec![]}
                        } else if scene.is_streamed()
                            && scene.generated_chunk((x >> 4, z >> 4)).is_none()
                        {
                            vec![[0., 0., 0., 1., 1., 1.]]
                        } else {
                            PlayerWorld::collision_boxes(&*scene, (x, y, z))
                        };
                        for shape in shapes {
                            let p = to_source(
                                self.origin,
                                [
                                    x as f64 + shape[0],
                                    y as f64 + shape[1],
                                    z as f64 + shape[2],
                                ],
                            );
                            let q = to_source(
                                self.origin,
                                [
                                    x as f64 + shape[3],
                                    y as f64 + shape[4],
                                    z as f64 + shape[5],
                                ],
                            );
                            collision::clip_bounds(
                                start,
                                end,
                                mins,
                                maxs,
                                p.min(q),
                                p.max(q),
                                contents::SOLID,
                                1,
                                &mut out,
                            );
                        }
                    }
                }
            }
            if out.all_solid || out.fraction <= (step + 1) as f32 / steps as f32 {
                break;
            }
        }
        out
    }
}
impl DynamicWorld for Collision {
    fn voxel_scale(&self)->f32 {BLOCK}
    fn trace_box(&self,start:Vec3,end:Vec3,mins:Vec3,maxs:Vec3,mask:i32)->Trace {self.trace_context(start,end,mins,maxs,mask,None)}
    fn trace_player(&self,start:Vec3,end:Vec3,mins:Vec3,maxs:Vec3,mask:i32,drop_scaffold:bool)->Trace {self.trace_context(start,end,mins,maxs,mask,Some(drop_scaffold))}
    fn climbable(&self,origin:Vec3,mins:Vec3,maxs:Vec3)->collision::Climbable {
        let a=to_block(self.origin,origin+Vec3::new(mins.x,mins.y,0.1));
        let b=to_block(self.origin,origin+Vec3::new(maxs.x,maxs.y,BLOCK*0.85));
        let scene=self.scene.read().unwrap();
        let mut result=collision::Climbable::None;
        for x in a[0].min(b[0]).floor() as i32..=a[0].max(b[0]).floor() as i32 {
            for z in a[2].min(b[2]).floor() as i32..=a[2].max(b[2]).floor() as i32 {
                for y in (a[1]-0.1).floor() as i32..=b[1].floor() as i32 {
                    if let Some(block)=Scene::block(&*scene,(x,y,z)) {
                        match block.id.path.as_str() {
                            "scaffolding"=>return collision::Climbable::Scaffolding,
                            "ladder"|"vine"|"weeping_vines"|"weeping_vines_plant"|"twisting_vines"|"twisting_vines_plant"|"cave_vines"|"cave_vines_plant" if y as f64+1.>a[1] =>result=collision::Climbable::Ladder,
                            _=>{}
                        }
                    }
                }
            }
        }
        result
    }
    fn point_contents(&self, p: Vec3, mask: i32) -> i32 {
        let b = block_pos(to_block(self.origin, p));
        let s = self.scene.read().unwrap();
        let water =
            Scene::block(&*s, b).is_some_and(|b| matches!(b.id.path.as_str(), "water" | "lava"));
        if water {
            contents::WATER & mask
        } else {
            drop(s);
            if self
                .trace_box(p, p, Vec3::ZERO, Vec3::ZERO, mask)
                .start_solid
            {
                contents::SOLID & mask
            } else {
                0
            }
        }
    }
    fn hit_target(&self, start: Vec3, end: Vec3) -> Option<(u64, f32)> {
        let d = end - start;
        let mut best = None;
        for &(key, lo, hi) in self.targets.read().unwrap().iter() {
            let (mut near, mut far) = (0.0f32, 1.0f32);
            let mut ok = true;
            for k in 0..3 {
                if d[k].abs() < 1e-6 {
                    if start[k] < lo[k] || start[k] > hi[k] {
                        ok = false;
                        break;
                    }
                } else {
                    let a = (lo[k] - start[k]) / d[k];
                    let b = (hi[k] - start[k]) / d[k];
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                    if near > far {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && best.is_none_or(|(_, t)| near < t) {
                best = Some((key, near));
            }
        }
        best
    }
    fn damage_target(&self, key: u64, damage: f32, from: Vec3) {
        let point = self.targets.read().unwrap().iter().find(|(id,_,_)| *id==key)
            .map_or(from, |(_,lo,hi)| (*lo+*hi)*0.5);
        self.damage_target_at(key, damage, from, point);
    }
    fn damage_target_at(&self, key:u64, damage:f32, from:Vec3, point:Vec3) {
        self.events.lock().unwrap().push(Interaction::Mob { key, damage, from, point });
    }
    fn damage_block(&self, point: Vec3, normal: Vec3, damage: f32) {
        self.events.lock().unwrap().push(Interaction::Shot {
            point,
            normal,
            damage,
        });
    }
}

pub struct Runtime {
    pub stream: TerrainStream,
    pub collision: Arc<Collision>,
    pub packs: PackStack,
    pub atlas: Arc<Atlas>,
    pub registries: Arc<Registries>,
    pub server: ServerHandle,
    pub save: Save,
    pub directory: PathBuf,
    pub mobs: mc_entities::world::EntityWorld,
    pub poses: terrain::client_mobs::ClientMobs,
    pub light: terrain::lighting::SkyLight,
    pub inventory: mc_player::inventory::Inventory,
    pub entity_snapshot: Option<terrain::server::EntitySnapshot>,
    pub item_visuals: terrain::mesh::ItemVisuals,
    pub player_hits: Vec<MobPlayerHit>,
    pub mob_kills: Vec<String>,
    pub mob_hits: Vec<MobHitFx>,
    pending_mob_hits: std::collections::VecDeque<MobHitFx>,
    pub adventure_hurt: HashMap<u64,f32>,
    pub stream_tick_ms:f32,
    pub stream_tick_loaded:usize,
    pub sounds: Vec<(String, [f64; 3])>,
    pub local_block_sounds: Vec<block_audio::Sound>,
    pub mining_hits: block_audio::MiningHits,
    sound_events: serde_json::Map<String,serde_json::Value>,
    pub damage: HashMap<BlockPos, f32>,
    pub clock: f32,
    save_clock: f32,
    pub mining: mc_player::mining::Mining,
    pub mine_clock: f32,
    pub eating: Option<mc_player::food::FoodUse>,
    pub screen: survival::Screen,
    pub epoch: u64,
    pub teleport: Option<Vec3>,
    pub portal_clock: f32,
    pub portal_cooldown: f32,
    pub notices: Vec<String>,
    pub was_alive: bool,
    pub dragon_model: dragon_model::DragonModel,
    /// Section meshes built by [`Runtime::pregenerate`] before the window
    /// opened, handed to the renderer with the first frame.
    preloaded: Option<Vec<(terrain::sections::SectionPos, terrain::mesh::SectionMesh)>>,
}

/// Progress of [`Runtime::pregenerate`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Pregen {
    /// Chunk columns within the render distance that are generated.
    pub chunks: usize,
    pub chunks_total: usize,
    /// Section meshes built so far.
    pub sections: usize,
    pub seconds: f32,
    /// Everything in range is generated, lit and built.
    pub done: bool,
}
impl Runtime {
    pub fn sound_bytes(&self, event: &str, variant: usize) -> Option<(String, Vec<u8>)> {
        let events = &self.sound_events;
        let mut name = event.trim_start_matches("minecraft:").to_string();
        for _ in 0..8 {
            let sounds = events.get(&name)?.get("sounds")?.as_array()?;
            let value = sounds.get(variant % sounds.len().max(1))?;
            let path = value.as_str().or_else(|| value.get("name")?.as_str())?;
            if value.get("type").and_then(|v| v.as_str()) == Some("event") {
                name = path.trim_start_matches("minecraft:").into();
                continue;
            }
            let id = terrain::pack::ResourceId::parse(path).ok()?;
            return Some((id.key(), self.packs.sound(&id).ok()??));
        }
        None
    }
    /// Build walking surfaces from live block collision, including one-block jumps.
    pub fn navigation(&self, center: Vec3, radius: i32) -> nav::NavMesh {
        let c = block_pos(to_block(self.save.origin, center));
        let scene = self.collision.scene.read().unwrap();
        let mut cells = HashMap::new();
        let mut areas = Vec::new();
        for x in c.0 - radius..=c.0 + radius {
            for z in c.2 - radius..=c.2 + radius {
                for y in c.1 - 12..=c.1 + 12 {
                    if scene.generated_chunk((x >> 4, z >> 4)).is_none() {
                        continue;
                    }
                    let boxes = PlayerWorld::collision_boxes(&*scene, (x, y, z));
                    let top = boxes
                        .iter()
                        .filter(|b| b[0] <= 0.05 && b[2] <= 0.05 && b[3] >= 0.95 && b[5] >= 0.95)
                        .map(|b| b[4])
                        .fold(0.0f64, f64::max);
                    if top < 0.5
                        || !PlayerWorld::collision_boxes(&*scene, (x, y + 1, z)).is_empty()
                        || !PlayerWorld::collision_boxes(&*scene, (x, y + 2, z)).is_empty()
                    {
                        continue;
                    }
                    let a = to_source(self.save.origin, [x as f64, y as f64 + top, z as f64]);
                    let b = to_source(
                        self.save.origin,
                        [x as f64 + 1., y as f64 + top, z as f64 + 1.],
                    );
                    let id = (((x as u32 & 1023) << 19)
                        | ((z as u32 & 1023) << 9)
                        | ((y + 64) as u32 & 511))
                        .wrapping_add(1);
                    cells.insert((x, y, z), areas.len());
                    areas.push(nav::Area {
                        id,
                        flags: 0,
                        nw: a.min(b),
                        se: a.max(b),
                        ne_z: a.z,
                        sw_z: a.z,
                        connections: Default::default(),
                        place: 0,
                        hiding_spots: vec![],
                    });
                }
            }
        }
        // A CS hull is slightly taller than two blocks after the floor epsilon.
        // Reject low canopies and partial blocks using the exact movement hull.
        drop(scene);
        let valid: std::collections::HashSet<_> = areas
            .iter()
            .filter(|a| {
                let p = a.center() + Vec3::Z * 0.1;
                !self
                    .collision
                    .trace_box(
                        p,
                        p,
                        Vec3::new(-16., -16., 0.),
                        Vec3::new(16., 16., 72.),
                        contents::MASK_PLAYERSOLID,
                    )
                    .start_solid
            })
            .map(|a| a.id)
            .collect();
        for (&(x, y, z), &i) in &cells {
            for (direction, (dx, dz)) in [(0, 1), (1, 0), (0, -1), (-1, 0)].into_iter().enumerate()
            {
                for dy in -2..=1 {
                    if let Some(&j) = cells.get(&(x + dx, y + dy, z + dz)) {
                        let target = areas[j].id;
                        if !valid.contains(&target) {
                            continue;
                        }
                        areas[i].connections[direction].push(target);
                        if dy > 0 {
                            areas[i].flags |= nav::attr::JUMP;
                        }
                        break;
                    }
                }
            }
        }
        areas.retain(|a| valid.contains(&a.id));
        nav::NavMesh::from_areas(areas)
    }
    pub fn load() -> Result<Self, String> {
        Self::load_seed(None)
    }
    pub fn load_seed(requested_seed: Option<i64>) -> Result<Self, String> {
        let root = asset_root()?;
        let seed = requested_seed
            .or_else(|| {
                std::env::var("CSR_WORLD_SEED")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(42);
        let directory = save_root().join(format!("Overworld-{seed}"));
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut saved = if directory.join("world.json").exists() {
            Some(
                serde_json::from_slice::<Save>(
                    &std::fs::read(directory.join("world.json")).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("Cannot read world save: {e}"))?,
            )
        } else {
            None
        };
        if saved
            .as_ref()
            .is_some_and(|s| !(1..=2).contains(&s.version) || s.seed != seed)
        {
            return Err("Unsupported world save version/seed".into());
        }
        if let Some(save) = &mut saved {
            if save.survival.respawn_pending {
                save.player = save.survival.spawn.unwrap_or(save.origin);
                save.adventure.dimension = "overworld".into();
                save.survival.health = 20.;
                save.survival.respawn_pending = false;
            }
        }
        let registries = Arc::new(Registries::load(&DataPaths::under(&root))?);
        let packs = PackStack::open(vec![root.join("resourcepacks/local/minecraft-26.3")])
            .map_err(|e| e.to_string())?;
        let distance = std::env::var("CSR_RENDER_DISTANCE")
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(6)
            .clamp(3, 16);
        let mut stream = TerrainStream::for_dimension(
            registries.clone(),
            seed,
            distance,
            saved
                .as_ref()
                .map(|s| s.adventure.dimension())
                .unwrap_or(Dimension::Overworld),
            Some(&directory),
        )
        .map_err(|e| e.to_string())?;
        let spawn = [
            stream.player_spawn.0,
            stream.player_spawn.1,
            stream.player_spawn.2,
        ];
        let mut save = saved.unwrap_or_else(|| Save {
            version: 1,
            seed,
            origin: spawn,
            player: spawn,
            day_ticks: 1000.,
            items: vec![],
            selected: 0,
            survival: Default::default(),
            adventure: Default::default(),
        });
        let atlas = terrain::mesh::build(&HandcraftedScene::default(), &packs)
            .map_err(|e| e.to_string())?
            .atlas;
        let mut scene = HandcraftedScene::streamed(stream.states.clone());
        let mut sim = ServerSim::new(
            stream.world_gen(),
            stream.states.clone(),
            save.adventure.dimension().dimension_type(),
        );
        sim.set_entity_storage(stream.storage());
        sim.load_loot(&root.join("client.jar"), seed as u64);
        let mut server = ServerHandle::spawn(sim);
        let mut inventory = save.survival.inventory.clone();
        if save.version == 1 {
            // Back up the legacy sandbox; Survival starts without its free supplies.
            if directory.join("world.json").exists()
                && !directory.join("world-v1-backup.json").exists()
            {
                std::fs::copy(
                    directory.join("world.json"),
                    directory.join("world-v1-backup.json"),
                )
                .map_err(|e| e.to_string())?;
            }
            save.version = 2;
        }
        save.items.clear();
        let book = Arc::new(survival::recipe_book(&root)?);
        server.set_recipe_book(book.clone());
        inventory.recipes = book;
        let now = Instant::now();
        let center = block_pos(save.player);
        loop {
            let (loaded, forgotten) = stream.server_tick(center, &mut scene);
            for c in loaded {
                server.load_chunk(&c);
            }
            for c in forgotten {
                server.unload_chunk(c);
            }
            if (-1..=1).all(|x| {
                (-1..=1).all(|z| {
                    scene
                        .generated_chunk(((center.0 >> 4) + x, (center.2 >> 4) + z))
                        .is_some()
                })
            }) {
                break;
            }
            if now.elapsed() > Duration::from_secs(180) {
                return Err("Timed out generating spawn chunks".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        eprintln!(
            "Overworld: {} block states, atlas {}x{}, spawn {:?}, save {}",
            registries.blocks.state_count(),
            atlas.pixels.width(),
            atlas.pixels.height(),
            save.player,
            directory.display()
        );
        let scene = Arc::new(RwLock::new(scene));
        let collision = Arc::new(Collision {
            scene,
            origin: save.origin,
            events: Mutex::new(Vec::new()),
            targets: RwLock::new(Vec::new()),
        });
        let light = terrain::lighting::SkyLight::streamed();
        let mobs = mc_entities::world::EntityWorld::default();
        let sound_events=packs.sound_events("minecraft").unwrap_or_default();
        let dragon_model = dragon_model::DragonModel::load(&root).unwrap_or_else(|e| {
            eprintln!("Ender dragon model unavailable ({e}); the dragon will be invisible.");
            dragon_model::DragonModel::empty()
        });
        let mut runtime = Self {
            sound_events,
            stream,
            collision,
            packs,
            atlas,
            registries,
            server,
            save,
            directory,
            mobs,
            poses: Default::default(),
            light,
            inventory,
            entity_snapshot: None,
            item_visuals: Default::default(),
            player_hits: vec![], mob_kills: vec![], mob_hits: vec![], pending_mob_hits: Default::default(), adventure_hurt: Default::default(), stream_tick_ms:0., stream_tick_loaded:0,
            sounds: vec![],
            local_block_sounds: vec![], mining_hits: Default::default(),
            damage: HashMap::new(),
            clock: 0.,
            save_clock: 0.,
            mining: Default::default(),
            mine_clock: 0.,
            eating: None,
            screen: survival::Screen::Closed,
            epoch: 0,
            teleport: None,
            portal_clock: 0.,
            portal_cooldown: 5.,
            notices: vec![],
            was_alive: true,
            dragon_model,
            preloaded: None,
        };
        if let Ok(loot) = mc_player::loot::LootBook::from_jar(&root.join("client.jar")) {
            runtime.mining.set_loot(loot);
        }
        runtime.mining.set_loot_seed(seed as u64);
        runtime.load_mining_catalog();
        let spawn = runtime.spawn_source();
        let min = Vec3::new(-16., -16., 0.);
        let max = Vec3::new(16., 16., 72.);
        if runtime
            .collision
            .trace_box(spawn, spawn, min, max, contents::MASK_PLAYERSOLID)
            .start_solid
        {
            let mut safe = None;
            'search: for ring in 0..=8i32 {
                for dz in [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 7, -7, 8, -8] {
                    for dx in -ring..=ring {
                        for dy in -ring..=ring {
                            if dx.abs().max(dy.abs()) != ring {
                                continue;
                            }
                            let candidate =
                                spawn + Vec3::new(dx as f32, dy as f32, dz as f32) * BLOCK;
                            let tr = runtime.collision.trace_box(
                                candidate,
                                candidate - Vec3::Z * BLOCK * 1.1,
                                min,
                                max,
                                contents::MASK_PLAYERSOLID,
                            );
                            if !tr.start_solid && tr.hit() && tr.normal.z > 0.7 {
                                safe = Some(tr.end + Vec3::Z * 0.1);
                                break 'search;
                            }
                        }
                    }
                }
            }
            let safe = safe.ok_or("Could not find a clear spawn for the CS:GO player hull")?;
            runtime.save.player = to_block(runtime.save.origin, safe - Vec3::Z * 0.5);
            eprintln!(
                "Adjusted Minecraft spawn to fit CS:GO hull: {:?}",
                runtime.save.player
            );
        }
        Ok(runtime)
    }
    pub fn spawn_source(&self) -> Vec3 {
        to_source(self.save.origin, self.save.player) + Vec3::Z * 0.5
    }
    pub fn set_block(&mut self, pos: BlockPos, block: Option<Block>) -> bool {
        let mut scene = self.collision.scene.write().unwrap();
        if scene.generated_chunk((pos.0 >> 4, pos.2 >> 4)).is_none() {
            return false;
        }
        scene.set(pos, block.clone());
        self.stream.record_edits(&scene, &[pos]);
        self.stream.mark_edited(&scene, &[pos]);
        self.server.player_edit(
            &scene,
            pos,
            if block.is_some() {
                PlayerEdit::Place
            } else {
                PlayerEdit::Break
            },
        );
        true
    }
    pub fn mine(&mut self, pos: BlockPos, amount: f32) -> bool {
        let block = {
            let s = self.collision.scene.read().unwrap();
            Scene::block(&*s, pos).cloned()
        };
        let Some(block) = block else { return false };
        let state = self.stream.states.state_of(&block);
        let hardness = state
            .map(|s| self.registries.blocks.state(s).destroy_speed)
            .unwrap_or(1.0);
        if hardness < 0.0 {
            return false;
        }
        let damage = self.damage.entry(pos).or_default();
        *damage += amount;
        if *damage < hardness.max(0.05) * 45.0 {
            return false;
        }
        self.damage.remove(&pos);
        if !self.set_block(pos, None) {
            return false;
        }
        self.server.spawn_item(
            &block.id.key(),
            1,
            None,
            [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5],
            [0., 0.12, 0.],
            5,
            0,
        );
        self.sounds.push((
            format!(
                "minecraft:block.{}.break",
                terrain::audio::block_sound_family(&block.id.key())
            ),
            [pos.0 as f64, pos.1 as f64, pos.2 as f64],
        ));
        true
    }
    pub fn explode(&mut self, center: Vec3, radius: f32) {
        let b = block_pos(to_block(self.save.origin, center));
        let r = radius.ceil() as i32;
        for x in -r..=r {
            for y in -r..=r {
                for z in -r..=r {
                    if (x * x + y * y + z * z) as f32 <= radius * radius {
                        self.mine((b.0 + x, b.1 + y, b.2 + z), 150.);
                    }
                }
            }
        }
    }
    pub fn set_render_distance(&mut self,distance:i32) {
        let scene=self.collision.scene.read().unwrap();
        if self.stream.set_view_distance(&scene,distance.clamp(3,16)) { self.epoch+=1; }
    }
    pub fn frame(
        &mut self,
        camera: &terrain::sections::CullCamera,
    ) -> terrain::terrain::FrameUpdate {
        let scene = self.collision.scene.read().unwrap();
        let mut out = self
            .stream
            .frame(&scene, camera, 0, &self.atlas, &self.packs);
        for (p, l) in &out.lights {
            self.light.set_chunk_column(*p, l.clone());
        }
        if let Some(mut preloaded) = self.preloaded.take() {
            // Built before the window opened; newer uploads follow them.
            preloaded.append(&mut out.uploads);
            out.uploads = preloaded;
        }
        out
    }

    /// Generates, lights and meshes everything within the render distance
    /// around the player before play starts, so the world does not stream
    /// in (and stutter) during the first seconds. The camera sweeps all
    /// directions because section building follows the view frustum.
    /// Returns when the stream settles or `timeout` passes.
    pub fn pregenerate(
        &mut self,
        fov_degrees: f32,
        aspect: f32,
        timeout: Duration,
        progress: &mut dyn FnMut(&Pregen),
    ) -> Pregen {
        let started = Instant::now();
        let eye = to_block(self.save.origin, self.spawn_source() + Vec3::Z * 64.0);
        let center = block_pos(self.save.player);
        let distance = self.stream.view_distance();
        let mut built: HashMap<terrain::sections::SectionPos, terrain::mesh::SectionMesh> = HashMap::new();
        let mut views = Vec::new();
        for pitch in [0.0f32, 40.0] {
            for k in 0..6 {
                views.push((pitch, k as f32 * 60.0));
            }
        }
        let mut quiet_sweeps = 0;
        let mut last_report = Instant::now();
        let mut status = Pregen::default();
        loop {
            {
                let mut scene = self.collision.scene.write().unwrap();
                let (loaded, forgotten) = self.stream.server_tick(center, &mut scene);
                for c in loaded {
                    self.server.load_chunk(&c);
                }
                for c in forgotten {
                    self.server.unload_chunk(c);
                }
            }
            let mut uploads = 0;
            for &(pitch, yaw) in &views {
                let (p, y) = (pitch.to_radians(), yaw.to_radians());
                // Source forward (cos p cos y, cos p sin y, -sin p) in block axes.
                let forward = [p.cos() * y.cos(), -p.sin(), -p.cos() * y.sin()];
                let camera = terrain::sections::CullCamera {
                    position: glam_mc::DVec3::from_array(eye),
                    forward: glam_mc::Vec3::from_array(forward),
                    fov_degrees,
                    aspect,
                    yaw_degrees: yaw,
                    pitch_degrees: pitch,
                };
                // Drains finished work a little at a time, so ask repeatedly.
                for _ in 0..8 {
                    let update = self.frame(&camera);
                    for pos in &update.removed {
                        built.remove(pos);
                    }
                    let n = update.uploads.len();
                    for (pos, mesh) in update.uploads {
                        built.insert(pos, mesh);
                    }
                    uploads += n;
                    if n == 0 {
                        break;
                    }
                }
            }
            let scene = self.collision.scene.read().unwrap();
            let (cx, cz) = (center.0 >> 4, center.2 >> 4);
            status.chunks_total = ((2 * distance + 1) * (2 * distance + 1)) as usize;
            status.chunks = (-distance..=distance)
                .flat_map(|x| (-distance..=distance).map(move |z| (x, z)))
                .filter(|&(x, z)| scene.generated_chunk((cx + x, cz + z)).is_some())
                .count();
            drop(scene);
            status.sections = built.len();
            status.seconds = started.elapsed().as_secs_f32();
            quiet_sweeps = if uploads == 0 && self.stream.is_settled() { quiet_sweeps + 1 } else { 0 };
            status.done = quiet_sweeps >= 3;
            if status.done || started.elapsed() > timeout {
                break;
            }
            if last_report.elapsed() > Duration::from_millis(500) {
                last_report = Instant::now();
                progress(&status);
            }
            if uploads == 0 {
                std::thread::sleep(Duration::from_millis(3));
            }
        }
        progress(&status);
        // Nearest first, so the view fills in from the player outwards.
        let mut meshes: Vec<_> = built.into_iter().filter(|(_, m)| !m.indices.is_empty()).collect();
        let d2 = |p: &terrain::sections::SectionPos| {
            let c = [p.0 as f64 * 16.0 + 8.0 - eye[0], p.1 as f64 * 16.0 + 8.0 - eye[1], p.2 as f64 * 16.0 + 8.0 - eye[2]];
            c[0] * c[0] + c[1] * c[1] + c[2] * c[2]
        };
        meshes.sort_by(|a, b| d2(&a.0).total_cmp(&d2(&b.0)));
        self.preloaded = Some(meshes);
        self.stream.set_generation_threads(play_generation_threads());
        status
    }

    /// Sections waiting to be handed to the renderer.
    pub fn preloaded_sections(&self) -> usize {
        self.preloaded.as_ref().map_or(0, Vec::len)
    }
    pub fn save_now(&mut self) -> Result<(), String> {
        self.stream.save_all();
        self.save.survival.inventory = self.inventory.clone();
        let bytes = serde_json::to_vec_pretty(&self.save).map_err(|e| e.to_string())?;
        let temp = self.directory.join("world.json.tmp");
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, self.directory.join("world.json")).map_err(|e| e.to_string())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.server.wait_idle();
        self.receive();
        if let Err(e) = self.save_now() {
            eprintln!("Overworld save failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> Collision {
        let mut scene = HandcraftedScene::default();
        scene.set((0, 0, 0), Some(Block::new("minecraft:stone")));
        Collision {
            scene: Arc::new(RwLock::new(scene)),
            origin: [0.; 3],
            events: Mutex::new(vec![]),
            targets: RwLock::new(vec![]),
        }
    }
    #[test]
    fn coordinate_roundtrip() {
        for p in [
            Vec3::ZERO,
            Vec3::new(-392., 710., -44.),
            Vec3::splat(12345.),
        ] {
            let origin = [-399., -62., 287.];
            assert!(to_source(origin, to_block(origin, p)).distance(p) < 0.001);
        }
    }
    #[test]
    fn block_edits_change_bullets_and_hulls() {
        let w = world();
        let a = Vec3::new(-72., -18., 18.);
        let b = Vec3::new(100., -18., 18.);
        let hit = w.trace_box(a, b, Vec3::ZERO, Vec3::ZERO, contents::MASK_SHOT);
        assert!(hit.hit());
        assert_eq!(hit.normal, -Vec3::X);
        assert!(!hit.start_solid);
        w.scene.write().unwrap().set((0, 0, 0), None);
        assert!(
            !w.trace_box(a, b, Vec3::ZERO, Vec3::ZERO, contents::MASK_SHOT)
                .hit()
        );
    }
    #[test]
    fn standing_on_floor_is_not_stuck() {
        let w = world();
        let a = Vec3::new(BLOCK * 0.5, -BLOCK * 0.5, BLOCK + 0.5);
        let t = w.trace_box(
            a,
            a - Vec3::Z * 5.,
            Vec3::new(-16., -16., 0.),
            Vec3::new(16., 16., 72.),
            contents::MASK_PLAYERSOLID,
        );
        assert!(t.hit());
        assert!(!t.start_solid);
        assert_eq!(t.normal, Vec3::Z);
    }
    #[test]
    fn standing_player_walks_through_two_block_tunnel() {
        let w = world();
        {
            let mut scene = w.scene.write().unwrap();
            for x in -2..=5 {
                for z in -1..=1 {
                    scene.set((x, 0, z), Some(Block::new("minecraft:stone")));
                    scene.set((x, 3, z), Some(Block::new("minecraft:stone")));
                }
            }
        }
        assert!((BLOCK / 72. - 0.55).abs() < 1e-6);
        let start = Vec3::new(-BLOCK, -BLOCK * 0.5, BLOCK + 0.1);
        let end = start + Vec3::X * BLOCK * 5.;
        assert!(
            !w.trace_box(
                start,
                end,
                Vec3::new(-16., -16., 0.),
                Vec3::new(16., 16., 72.),
                contents::MASK_PLAYERSOLID
            )
            .hit()
        );
        w.scene
            .write()
            .unwrap()
            .set((1, 2, 0), Some(Block::new("minecraft:stone")));
        assert!(
            w.trace_box(
                start,
                end,
                Vec3::new(-16., -16., 0.),
                Vec3::new(16., 16., 72.),
                contents::MASK_PLAYERSOLID
            )
            .hit()
        );
    }
    #[test]
    fn mob_rays_stop_at_the_nearest_body() {
        let w = world();
        *w.targets.write().unwrap() = vec![
            (1, Vec3::splat(10.), Vec3::splat(20.)),
            (2, Vec3::splat(30.), Vec3::splat(40.)),
        ];
        assert_eq!(
            w.hit_target(Vec3::splat(0.), Vec3::splat(100.)).unwrap().0,
            1
        );
        assert!(w.hit_target(Vec3::splat(-50.), Vec3::splat(5.)).is_none());
    }
}

impl Runtime {
    pub fn tick(&mut self, dt: f32, feet: Vec3, alive: bool) {
        self.adventure_hurt.retain(|_,time| { *time -= dt; *time > 0. });
        let interactions = std::mem::take(&mut *self.collision.events.lock().unwrap());
        for action in interactions {
            match action {
                Interaction::Shot {
                    point,
                    normal,
                    damage,
                } => {
                    let pos = block_pos(to_block(self.save.origin, point - normal * 1.0));
                    let _ = (pos, damage); // Survival resources require the appropriate held tool.
                }
                Interaction::Mob { key, damage, from, point } => {
                    let before = self.mob_hits.len();
                    if self.hit_adventure(key, damage) {
                        if self.mob_hits.len() > before { *self.mob_hits.last_mut().unwrap() = MobHitFx { point, from }; }
                        continue;
                    }
                    if let Some(hit) = mobs::decode(key) {
                        let attack = mc_entities::world::PlayerAttack {
                            player_id: 1,
                            position: glam_mc::DVec3::from_array(to_block(self.save.origin, from)),
                            yaw: 0.,
                            attack_damage: (damage * 0.2) as f64,
                            strength: 1.,
                            sprinting: false,
                            can_critical: false,
                            can_sweep: false,
                        };
                        self.pending_mob_hits.push_back(MobHitFx { point, from });
                        self.server.mob_action(
                            hit,
                            Some(attack),
                            &self.inventory,
                            self.save.selected,
                            false,
                        );
                    }
                }
            }
        }
        self.save.player = to_block(self.save.origin, feet);
        self.clock += dt;
        self.save_clock += dt;
        if self.clock >= 0.05 {
            self.clock -= 0.05;
            self.save.day_ticks += 1.;
            self.poses.tick();
            let p = self.save.player;
            let center = block_pos(p);
            {
                let mut scene = self.collision.scene.write().unwrap();
                let started=std::time::Instant::now();
                let (loaded, forgotten) = self.stream.server_tick(center, &mut scene);
                self.stream_tick_loaded=loaded.len();
                for c in loaded {
                    self.server.load_chunk(&c);
                }
                for c in forgotten {
                    self.server.unload_chunk(c);
                }
                self.stream_tick_ms=started.elapsed().as_secs_f32()*1000.;
            }
            let candidate = mc_entities::tempt::PlayerCandidate {
                id: 1,
                position: glam_mc::DVec3::from_array(p),
                eye_height: 64.0 / BLOCK,
                main_hand_cow_food: false,
                offhand_cow_food: false,
                main_hand_pig_food: false,
                offhand_pig_food: false,
                main_hand_chicken_food: false,
                offhand_chicken_food: false,
                main_hand_carrot_on_a_stick: false,
                offhand_carrot_on_a_stick: false,
                main_hand_wolf_interest: false,
                offhand_wolf_interest: false,
                main_hand_horse_tempt: false,
                offhand_horse_tempt: false,
                alive,
                spectator: !alive,
                attackable: alive,
            };
            self.tick_furnaces();
            self.server.tick(terrain::server::TickInput {
                day_ticks: self.save.day_ticks as i64,
                players: vec![p],
                difficulty: 2,
                simulation_center: (center.0 >> 4, center.2 >> 4),
                simulation_distance: 4,
                pickup: alive.then(|| {
                    (
                        p,
                        Box::new(self.inventory.clone()),
                        self.save.selected.min(8),
                    )
                }),
                mob_players: vec![candidate],
                mob_views: vec![],
                mob_vitals: vec![],
                bright_outside: self.daylight() > 0.5,
                tracking: (p, 128.),
                player_hurts: vec![],
                spawn_mobs: true,
            });
        }
        self.receive();
        self.tick_adventure(dt, alive);
        if self.save_clock > 30. {
            self.save_clock = 0.;
            if let Err(e) = self.save_now() {
                eprintln!("Overworld save failed: {e}");
            }
        }
    }
    fn receive(&mut self) {
        for output in self.server.poll() {
            // Resolve before replacing snapshots: an exploding creeper may already be gone.
            for hit in &output.player_hits {
                let kind=hit.source.and_then(|id| output.mobs.as_ref().and_then(|w|w.entity_type(id)).or_else(||self.mobs.entity_type(id)));
                self.player_hits.push(MobPlayerHit::native(*hit,kind));
            }
            for kind in &output.player_kills { self.reward_mob(kind); }
            if !output.changes.is_empty() {
                let mut scene = self.collision.scene.write().unwrap();
                let mut positions = Vec::new();
                for (pos, block) in output.changes {
                    scene.set(pos, block);
                    positions.push(pos);
                }
                self.stream.record_edits(&scene, &positions);
                self.stream.mark_edited(&scene, &positions);
            }
            if let Some(entities) = output.entities {
                self.entity_snapshot = Some(entities);
            }
            if let Some(mobs) = output.mobs {
                self.mobs = *mobs;
                self.poses
                    .receive(terrain::client_mobs::server_mobs(&self.mobs));
                *self.collision.targets.write().unwrap() = mobs::boxes(&self.mobs)
                    .into_iter()
                    .map(|(id, b)| {
                        let a = to_source(self.save.origin, [b[0], b[1], b[2]]);
                        let c = to_source(self.save.origin, [b[3], b[4], b[5]]);
                        (id, a.min(c), a.max(c))
                    })
                    .collect();
            }

            for (_, _, id, count, components) in output.picked {
                let mut stack = mc_player::inventory::ItemStack::new(id, count.clamp(0, 64) as u8);
                stack.components = components
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok());
                if let Some(rest) = self.inventory.add_item(stack, self.save.selected.min(8)) {
                    self.drop_stack(rest, self.save.player);
                }
            }
            for sound in output.mob_sounds {
                self.sounds.push((sound.event, sound.position.to_array()));
            }
            for result in output.mob_results {
                if let Some(hurt) = result.attack_hurt {
                    if let Some(hit) = self.pending_mob_hits.pop_front() {
                        if hurt { self.mob_hits.push(hit); }
                    }
                }
                for sound in result.sounds {
                    self.sounds.push((sound.event, sound.position.to_array()));
                }
            }
        }
    }
    pub fn add_item(&mut self, id: String, mut count: u32) {
        while count > 0 {
            let n = count.min(64) as u8;
            if let Some(rest) = self.inventory.add_item(
                mc_player::inventory::ItemStack::new(&id, n),
                self.save.selected.min(8),
            ) {
                self.drop_stack(rest, self.save.player);
            }
            count -= n as u32;
        }
    }
    pub fn daylight(&self) -> f32 {
        if self.save.adventure.dimension == "nether" {
            return 0.;
        }
        if self.save.adventure.dimension == "end" {
            return 0.25;
        }
        terrain::day_cycle::DayCycle {
            ticks: self.save.day_ticks,
            paused: false,
        }
        .sky_light_level()
            / 15.0
    }
    pub fn mob_mesh(&mut self) -> terrain::mesh::ChunkMesh { self.mob_mesh_for(None) }
    pub fn mob_mesh_for(&mut self, view: Option<&rendering::EntityView>) -> terrain::mesh::ChunkMesh {
        use crate::terrain::*;
        let mut mesh = mesh::ChunkMesh::default();
        let w = &self.mobs;
        let p = &self.poses;
        let a = &self.atlas;
        let l = &self.light;
        let t = (self.clock / 0.05).clamp(0., 1.);
        let visible = |id| view.is_none_or(|v| p.pose(id,t).is_none_or(|pose| v.visible(pose.feet.to_array(), 4.)));
        cow_render::append_cows(&mut mesh, w.cows().iter().filter(|e| visible(e.id)), p, a, l, t);
        sheep_render::append_sheep(&mut mesh, w.sheep().iter().filter(|e| visible(e.id)), p, a, l, t);
        pig_render::append_pigs(&mut mesh, w.pigs().iter().filter(|e| visible(e.id)), p, a, l, t);
        chicken_render::append_chickens(&mut mesh, w.chickens().iter().filter(|e| visible(e.id)), p, a, l, t);
        bat_render::append_bats(&mut mesh, w.bats().iter().filter(|e| visible(e.id)), p, a, l, t);
        zombie_render::append_zombies(&mut mesh, w.zombies().iter().filter(|e| visible(e.id)), p, a, l, t);
        creeper_render::append_creepers(&mut mesh, w.creepers().iter().filter(|e| visible(e.id)), p, a, l, t);
        spider_render::append_spiders(&mut mesh, w.spiders().iter().filter(|e| visible(e.id)), p, a, l, t);
        skeleton_render::append_skeletons(&mut mesh, w.skeletons().iter().filter(|e| visible(e.id)), p, a, l, t);
        let scene = self.collision.scene.read().unwrap();
        villager_render::append_villagers(
            &mut mesh,
            w.villagers().iter().filter(|e| visible(e.id)),
            p,
            &|pos| Scene::block(&*scene, pos).and_then(|b| b.properties.get("facing").cloned()),
            a,
            l,
            t,
        );
        enderman_render::append_endermen(
            &mut mesh,
            w.endermen().iter().filter(|e| visible(e.id)),
            p,
            a,
            l,
            t,
            self.save.day_ticks as u64,
        );
        witch_render::append_witches(&mut mesh, w.witches().iter().filter(|e| visible(e.id)), p, a, l, t);
        golem_render::append_iron_golems(&mut mesh, w.iron_golems().iter().filter(|e| visible(e.id)), p, a, l, t);
        wolf_render::append_wolves(&mut mesh, w.wolves().iter().filter(|e| visible(e.id)), p, a, l, t, w.game_time());
        let mut translucent = mesh::ChunkMesh::default();
        horse_render::append_horses(&mut mesh, &mut translucent, w.cows().iter().filter(|e| visible(e.id)), p, a, l, t);
        slime_render::append_slimes(&mut mesh, &mut translucent, w.slimes().iter().filter(|e| visible(e.id)), p, a, l, t);
        let offset = mesh.vertices.len() as u32;
        mesh.vertices.extend(translucent.vertices);
        mesh.indices
            .extend(translucent.indices.into_iter().map(|i| i + offset));
        if let Some(snapshot) = &self.entity_snapshot {
            let mut items = mc_player::items::WorldItems::default();
            items.entities = snapshot
                .items
                .iter()
                .filter(|i| view.is_none_or(|v| v.visible(i.position, 2.)))
                .map(|i| mc_player::items::ItemEntity {
                    entity_id: i.id as u32,
                    stack: mc_player::inventory::ItemStack::new(
                        i.item.clone(),
                        i.count.clamp(0, 64) as u8,
                    ),
                    position: glam_mc::DVec3::from_array(i.position),
                    previous_position: glam_mc::DVec3::from_array(i.previous_position),
                    velocity: glam_mc::DVec3::from_array(i.velocity),
                    age: i.age.max(0) as u32,
                    bob_offset: i.id as f32,
                    pickup_delay: i.pickup_delay.max(0) as u16,
                    on_ground: i.on_ground,
                })
                .collect();
            if let Ok(items) = self.item_visuals.mesh(&items, &self.packs, a, l, t) {
                let offset = mesh.vertices.len() as u32;
                mesh.vertices.extend(items.vertices);
                mesh.indices
                    .extend(items.indices.into_iter().map(|i| i + offset));
            }
            let tnt: Vec<_> = snapshot
                .tnt
                .iter()
                .filter(|b| view.is_none_or(|v| v.visible(b.position, 2.)))
                .map(|b| {
                    (
                        glam_mc::Vec3::from_array(b.position.map(|x| x as f32)),
                        b.fuse as f32,
                    )
                })
                .collect();
            let _ = self
                .item_visuals
                .append_primed_tnt(&mut mesh, &tnt, &self.packs, a, l);
            let blocks: Vec<_> = snapshot
                .falling
                .iter()
                .filter(|b| view.is_none_or(|v| v.visible(b.position, 2.)))
                .map(|b| {
                    (
                        glam_mc::Vec3::from_array(b.position.map(|x| x as f32)),
                        b.block.clone(),
                    )
                })
                .collect();
            let _ = self
                .item_visuals
                .append_falling_blocks(&mut mesh, &blocks, &self.packs, a, l);
        }
        drop(scene);
        self.append_adventure_mesh(&mut mesh,view);
        mesh
    }
}

#[cfg(test)]
mod climbing_tests {
    use super::*;
    use collision::{Climbable,DynamicWorld};
    fn scene(block:&str)->Collision {
        let mut scene=HandcraftedScene::default();
        scene.set((0,0,0),Some(Block::new(block)));
        Collision {scene:Arc::new(RwLock::new(scene)),origin:[0.;3],events:Mutex::new(vec![]),targets:RwLock::new(vec![])}
    }
    #[test]
    fn climbable_blocks_are_detected_but_stone_is_not() {
        for id in ["ladder","vine","weeping_vines","weeping_vines_plant","twisting_vines","twisting_vines_plant","cave_vines","cave_vines_plant","scaffolding","stone"] {
            let world=scene(&format!("minecraft:{id}"));
            let p=to_source([0.;3],[0.5,0.1,0.5]);
            let kind=world.climbable(p,Vec3::new(-16.,-16.,0.),Vec3::new(16.,16.,72.));
            assert_eq!(kind,match id {"stone"=>Climbable::None,"scaffolding"=>Climbable::Scaffolding,_=>Climbable::Ladder},"{id}");
        }
    }
    #[test]
    fn vines_have_no_solid_cube_and_scaffolding_has_one_way_top() {
        let lo=Vec3::new(-10.,-10.,0.);let hi=Vec3::new(10.,10.,72.);
        for id in ["vine","weeping_vines","twisting_vines"] {
            let w=scene(&format!("minecraft:{id}"));
            let a=to_source([0.;3],[-1.,0.,0.5]);let b=to_source([0.;3],[2.,0.,0.5]);
            assert!(!w.trace_player(a,b,lo,hi,contents::MASK_SOLID,false).hit(),"{id} was solid");
        }
        let w=scene("minecraft:scaffolding");
        let above=to_source([0.;3],[0.5,1.2,0.5]);let below=to_source([0.;3],[0.5,0.,0.5]);
        assert!(w.trace_player(above,below,lo,hi,contents::MASK_SOLID,false).hit());
        assert!(!w.trace_player(above,below,lo,hi,contents::MASK_SOLID,true).hit());
        assert!(!w.trace_player(below,above,lo,hi,contents::MASK_SOLID,false).hit());
    }
    #[test]
    fn native_ladder_and_scaffold_movement_respects_ceiling_and_sneak() {
        for scaffold in [false,true] {
            let c=scene("minecraft:air");
            {
                let mut s=c.scene.write().unwrap();
                for y in 0..7 {
                    s.set((0,y,0),Some(if scaffold {Block::new("minecraft:scaffolding")} else {Block::new("minecraft:ladder").with("facing","west")}));
                    s.set((1,y,0),Some(Block::new("minecraft:stone")));
                }
                for x in -1..3 {s.set((x,-1,0),Some(Block::new("minecraft:stone")));s.set((x,7,0),Some(Block::new("minecraft:stone")));}
            }
            let mut world=collision::CollisionWorld::new();world.dynamic=Some(Arc::new(c));
            let mut p=movement::PlayerMove::default();p.origin=to_source([0.;3],[0.38,0.01,0.5]);p.on_ground=true;
            let mut input=movement::MoveInput {forward_move:if scaffold {0.} else {450.},buttons:if scaffold {movement::buttons::JUMP} else {0},..Default::default()};
            for _ in 0..128 {movement::player_move(&mut p,&input,&Default::default(),&world,1./64.);}
            assert!(p.origin.z>BLOCK*3.,"failed to climb: scaffold={scaffold} {:?}",p.origin);
            assert!(p.origin.z+72.<BLOCK*7.+0.2,"passed through ceiling");
            input.forward_move=0.;input.buttons=movement::buttons::DUCK;
            for _ in 0..16 {movement::player_move(&mut p,&input,&Default::default(),&world,1./64.);}
            let held=p.origin.z;
            for _ in 0..32 {movement::player_move(&mut p,&input,&Default::default(),&world,1./64.);}
            if scaffold {assert!(p.origin.z<held-BLOCK,"scaffold crouch should descend");}
            else {assert!((p.origin.z-held).abs()<0.1,"ladder crouch should hold height");}
        }
    }

}

/// A player hit keeps the attacker identity even if that entity despawns afterward.
pub struct MobPlayerHit {
    pub hit: mc_entities::world::PlayerHit,
    pub attacker: String,
    pub method: &'static str,
}
impl MobPlayerHit {
    pub fn native(hit: mc_entities::world::PlayerHit, kind: Option<&str>) -> Self {
        use mc_entities::world::PlayerHitKind;
        let method=match hit.kind {PlayerHitKind::Melee{..}=>"melee",PlayerHitKind::Arrow{..}=>"arrow",PlayerHitKind::Explosion{..}=>"explosion"};
        Self {hit,attacker:kind.unwrap_or("minecraft:mob").into(),method}
    }
    pub fn cause(&self)->String {format!("mob:{}|{}",self.attacker,self.method)}
}

#[cfg(test)]
mod mob_feedback_tests {
    use super::*;
    #[test]
    fn accepted_native_damage_flashes_for_ten_ticks_and_rejected_hits_stay_silent() {
        use terrain::{client_mobs::{ClientMobs,server_mobs},mob_actions::{attack,Actor}};
        let mut world=mc_entities::world::EntityWorld::default();
        let id=world.spawn_cow(mc_entities::cow::Cow::new(glam_mc::DVec3::ZERO),true);
        let mut poses=ClientMobs::default();
        poses.receive(server_mobs(&world));
        assert_eq!(poses.pose(id,0.).unwrap().overlay(0.),1.);
        let mut inventory=mc_player::inventory::Inventory::default();
        let mut actor=Actor{inventory:&mut inventory,selected:0,infinite:false,entity_loot:None,shearing_loot:None};
        let hit=mc_entities::world::PlayerAttack{player_id:1,position:glam_mc::DVec3::X*3.,yaw:0.,attack_damage:1.,strength:1.,sprinting:false,can_critical:false,can_sweep:false};
        assert!(attack(&mut world,mc_entities::world::MobHit::Cow(id),&mut actor,&hit).hurt);
        poses.receive(server_mobs(&world));
        assert!(poses.pose(id,0.).unwrap().overlay(0.)<0.);
        assert!(!attack(&mut world,mc_entities::world::MobHit::Cow(id),&mut actor,&hit).hurt,"invulnerable follow-up must not produce blood");
        for _ in 0..10 { poses.tick(); }
        assert_eq!(poses.pose(id,0.).unwrap().overlay(0.),1.,"hurt overlay expires");
        let fatal=mc_entities::world::PlayerAttack{attack_damage:100.,..hit};
        assert!(attack(&mut world,mc_entities::world::MobHit::Cow(id),&mut actor,&fatal).hurt);
        assert!(!attack(&mut world,mc_entities::world::MobHit::Cow(id),&mut actor,&fatal).hurt,"corpse hits must not bleed");
    }
}
