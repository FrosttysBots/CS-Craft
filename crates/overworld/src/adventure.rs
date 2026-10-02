//! Dimension travel and the Survival completion path. Clean-room host gameplay.
use crate::*;
use mc_player::inventory::ItemStack;
const BOSS: u64 = 0xf000_0000_0000_0000;
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Adventure {
    pub dimension: String,
    pub returns: HashMap<String, [f64; 3]>,
    pub stronghold: Option<[i32; 3]>,
    pub dragon: Dragon,
    pub blazes: Vec<Blaze>,
    pub defeated: bool,
    pub completed: bool,
    pub arena_built: bool,
    pub next_id: u64,
    pub projectiles: Vec<Projectile>,
}
impl Default for Adventure {
    fn default() -> Self {
        Self {
            dimension: "overworld".into(),
            returns: HashMap::new(),
            stronghold: None,
            dragon: Dragon::default(),
            blazes: vec![],
            defeated: false,
            completed: false,
            arena_built: false,
            next_id: 100,
            projectiles: vec![],
        }
    }
}
impl Adventure {
    pub fn dimension(&self) -> Dimension {
        Dimension::parse(&self.dimension).unwrap_or_default()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Dragon {
    pub health: f32,
    pub position: [f64; 3],
    pub phase_time: f32,
    pub crystals: Vec<Crystal>,
    pub death_time: f32,
}
impl Default for Dragon {
    fn default() -> Self {
        Self {
            health: 200.,
            position: [0., 85., 35.],
            phase_time: 0.,
            crystals: vec![],
            death_time: 0.,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Crystal {
    pub position: [f64; 3],
    pub alive: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Blaze {
    pub id: u64,
    pub position: [f64; 3],
    pub health: f32,
    pub cooldown: f32,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Projectile {
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub life: f32,
    pub kind: u8,
}
impl Runtime {
    pub fn block_name(&self, p: BlockPos) -> Option<String> {
        let s = self.collision.scene.read().unwrap();
        Scene::block(&*s, p).map(|b| b.id.path.clone())
    }
    pub fn change_dimension(
        &mut self,
        dimension: Dimension,
        target: Option<[f64; 3]>,
    ) -> Result<Vec3, String> {
        let portal_arrival = self.save.adventure.dimension != "end" && self.was_alive;
        self.close_screen();
        self.server.wait_idle();
        self.receive();
        self.save_now()?;
        let distance = self.stream.view_distance();
        let mut stream = TerrainStream::for_dimension(
            self.registries.clone(),
            self.save.seed,
            distance,
            dimension,
            Some(&self.directory),
        )
        .map_err(|e| e.to_string())?;
        let spawn = [
            stream.player_spawn.0,
            stream.player_spawn.1,
            stream.player_spawn.2,
        ];
        let pos = target.unwrap_or(spawn);
        let mut scene = HandcraftedScene::streamed(stream.states.clone());
        let mut sim = ServerSim::new(
            stream.world_gen(),
            stream.states.clone(),
            dimension.dimension_type(),
        );
        sim.set_entity_storage(stream.storage());
        sim.load_loot(&asset_root()?.join("client.jar"), self.save.seed as u64);
        let mut server = ServerHandle::spawn(sim);
        server.set_recipe_book(self.inventory.recipes.clone());
        let center = if dimension == Dimension::End {
            (0, 64, 0)
        } else {
            block_pos(pos)
        };
        let radius = if dimension == Dimension::End { 3 } else { 1 };
        let now = Instant::now();
        loop {
            let (loaded, forgotten) = stream.server_tick(center, &mut scene);
            for c in loaded {
                server.load_chunk(&c);
            }
            for c in forgotten {
                server.unload_chunk(c);
            }
            if (-radius..=radius).all(|x| {
                (-radius..=radius).all(|z| {
                    scene
                        .generated_chunk(((center.0 >> 4) + x, (center.2 >> 4) + z))
                        .is_some()
                })
            }) {
                break;
            }
            if now.elapsed() > Duration::from_secs(180) {
                return Err("Timed out preparing dimension".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        stream.set_generation_threads(crate::play_generation_threads());
        self.stream = stream;
        self.server = server;
        *self.collision.scene.write().unwrap() = scene;
        self.collision.targets.write().unwrap().clear();
        self.collision.events.lock().unwrap().clear();
        self.save
            .adventure
            .returns
            .insert(self.save.adventure.dimension.clone(), self.save.player);
        self.save.adventure.dimension = match dimension {
            Dimension::Overworld => "overworld",
            Dimension::Nether => "nether",
            Dimension::End => "end",
        }
        .into();
        self.save.player = pos;
        self.mobs = Default::default();
        self.poses = Default::default();
        self.light = terrain::lighting::SkyLight::streamed();
        self.entity_snapshot = None;
        self.item_visuals = Default::default();
        self.save.adventure.projectiles.clear();
        self.damage.clear();
        self.mining.reset();
        self.mining_hits.tick(None);
        self.local_block_sounds.clear();
        self.player_hits.clear();
        self.mob_hits.clear();
        self.pending_mob_hits.clear();
        self.adventure_hurt.clear();
        self.epoch += 1;
        self.portal_clock = 0.;
        self.portal_cooldown = 5.;
        self.save.survival.fall = 0.;
        self.save.survival.last_y = pos[1];
        if dimension == Dimension::End {
            // Arrival platform remains inside the loaded arena; clear a safe 3-block-high pocket.
            let p = [60.5, 70.02, 0.5];
            self.save.player = p;
            for x in 58..=62 {
                for z in -2..=2 {
                    for y in 69..=73 {
                        self.set_block(
                            (x, y, z),
                            if y == 69 {
                                Some(Block::new("minecraft:obsidian"))
                            } else {
                                None
                            },
                        );
                    }
                }
            }
            if !self.save.adventure.arena_built {
                self.build_end_arena();
            }
        } else {
            let base = block_pos(self.save.player);
            let mut safe = None;
            for dy in 0..128 {
                let y = (base.1 + dy).min(if dimension == Dimension::Nether {
                    120
                } else {
                    310
                });
                let p = (base.0, y, base.2);
                if self.block_name(p).is_none()
                    && self.block_name((p.0, y + 1, p.2)).is_none()
                    && self
                        .block_name((p.0, y - 1, p.2))
                        .is_some_and(|b| b != "lava" && b != "water")
                {
                    safe = Some([p.0 as f64 + 0.5, y as f64 + 0.02, p.2 as f64 + 0.5]);
                    break;
                }
            }
            if let Some(p) = safe {
                self.save.player = p;
            }
            if portal_arrival {
                if let Some(p) = self.existing_portal_arrival() {
                    self.save.player = p;
                } else {
                    self.build_arrival_portal();
                }
            }
        }
        let feet = to_source(self.save.origin, self.save.player) + Vec3::Z * 0.5;
        self.teleport = Some(feet);
        self.save_now()?;
        Ok(feet)
    }
    fn existing_portal_arrival(&self) -> Option<[f64; 3]> {
        let center = block_pos(self.save.player);
        let scene = self.collision.scene.read().unwrap();
        let mut candidates = vec![];
        for dx in -16..=16 {
            for dz in -16..=16 {
                for dy in -16..=16 {
                    let p = (center.0 + dx, center.1 + dy, center.2 + dz);
                    let Some(block) =
                        Scene::block(&*scene, p).filter(|b| b.id.path == "nether_portal")
                    else {
                        continue;
                    };
                    let offsets = if block.properties.get("axis").map(String::as_str) == Some("z") {
                        [(1, 0), (-1, 0)]
                    } else {
                        [(0, 1), (0, -1)]
                    };
                    for (ox, oz) in offsets {
                        let q = (p.0 + ox, p.1, p.2 + oz);
                        if PlayerWorld::collision_boxes(&*scene, q).is_empty()
                            && PlayerWorld::collision_boxes(&*scene, (q.0, q.1 + 1, q.2)).is_empty()
                            && !PlayerWorld::collision_boxes(&*scene, (q.0, q.1 - 1, q.2))
                                .is_empty()
                        {
                            candidates.push((
                                dx * dx + dy * dy + dz * dz,
                                [q.0 as f64 + 0.5, q.1 as f64 + 0.02, q.2 as f64 + 0.5],
                            ));
                        }
                    }
                }
            }
        }
        candidates
            .into_iter()
            .min_by_key(|(d, _)| *d)
            .map(|(_, p)| p)
    }
    fn build_arrival_portal(&mut self) {
        let (x, y, z) = block_pos(self.save.player);
        // Keep the player in front of the portal, away from its immediate trigger.
        for dx in -2..=3 {
            for dz in -2..=2 {
                self.set_block(
                    (x + dx, y - 1, z + dz),
                    Some(Block::new("minecraft:obsidian")),
                );
                for dy in 0..=3 {
                    self.set_block((x + dx, y + dy, z + dz), None);
                }
            }
        }
        for dx in -1..=2 {
            for dy in -1..=3 {
                let edge = dx == -1 || dx == 2 || dy == -1 || dy == 3;
                let mut b = Block::new(if edge {
                    "minecraft:obsidian"
                } else {
                    "minecraft:nether_portal"
                });
                if !edge {
                    b.properties.insert("axis".into(), "x".into());
                }
                self.set_block((x + dx, y + dy, z + 1), Some(b));
            }
        }
    }
    pub fn ignite_portal(&mut self, p: BlockPos) -> bool {
        for axis in 0..2 {
            let at = |x: i32, y: i32| {
                if axis == 0 {
                    (p.0 + x, p.1 + y, p.2)
                } else {
                    (p.0, p.1 + y, p.2 + x)
                }
            };
            for bottom in -20..=0 {
                for left in -20..=0 {
                    for width in 2..=21 {
                        if !self
                            .block_name(at(left - 1, bottom))
                            .is_some_and(|b| b == "obsidian")
                        {
                            continue;
                        }
                        for height in 3..=21 {
                            let frame = (0..width).all(|x| {
                                self.block_name(at(left + x, bottom - 1)).as_deref()
                                    == Some("obsidian")
                                    && self.block_name(at(left + x, bottom + height)).as_deref()
                                        == Some("obsidian")
                            }) && (0..height).all(|y| {
                                self.block_name(at(left - 1, bottom + y)).as_deref()
                                    == Some("obsidian")
                                    && self.block_name(at(left + width, bottom + y)).as_deref()
                                        == Some("obsidian")
                            });
                            if !frame {
                                continue;
                            }
                            if !(0..width).all(|x| {
                                (0..height).all(|y| {
                                    self.block_name(at(left + x, bottom + y))
                                        .is_none_or(|s| s == "fire" || s == "nether_portal")
                                })
                            }) {
                                continue;
                            }
                            for x in 0..width {
                                for y in 0..height {
                                    let mut b = Block::new("minecraft:nether_portal");
                                    b.properties.insert(
                                        "axis".into(),
                                        if axis == 0 { "x" } else { "z" }.into(),
                                    );
                                    self.set_block(at(left + x, bottom + y), Some(b));
                                }
                            }
                            self.sounds.push((
                                "item.flintandsteel.use".into(),
                                [p.0 as f64, p.1 as f64, p.2 as f64],
                            ));
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    pub fn insert_eye(&mut self, p: BlockPos) -> bool {
        let mut b = {
            let s = self.collision.scene.read().unwrap();
            Scene::block(&*s, p).cloned()
        };
        let Some(ref mut block) = b else { return false };
        if block.id.path != "end_portal_frame"
            || block.properties.get("eye").is_some_and(|v| v == "true")
        {
            return false;
        }
        block.properties.insert("eye".into(), "true".into());
        self.set_block(p, b);
        self.consume_selected();
        for dx in -2..=2 {
            for dz in -2..=2 {
                let c = (p.0 + dx, p.1, p.2 + dz);
                if self.end_frame_complete(c) {
                    for x in -1..=1 {
                        for z in -1..=1 {
                            self.set_block(
                                (c.0 + x, c.1, c.2 + z),
                                Some(Block::new("minecraft:end_portal")),
                            );
                        }
                    }
                    self.notices.push("The End portal is open".into());
                    return true;
                }
            }
        }
        true
    }
    pub fn end_frame_complete(&self, c: BlockPos) -> bool {
        let s = self.collision.scene.read().unwrap();
        for i in -1..=1 {
            for (dx, dz, facing) in [
                (i, -2, "south"),
                (i, 2, "north"),
                (-2, i, "east"),
                (2, i, "west"),
            ] {
                let Some(b) = Scene::block(&*s, (c.0 + dx, c.1, c.2 + dz)) else {
                    return false;
                };
                if b.id.path != "end_portal_frame"
                    || b.properties.get("eye").map(String::as_str) != Some("true")
                    || b.properties.get("facing").map(String::as_str) != Some(facing)
                {
                    return false;
                }
            }
        }
        true
    }
    pub fn locate_stronghold(&mut self) -> Option<[i32; 3]> {
        if let Some(p) = self.save.adventure.stronghold {
            return Some(p);
        }
        if self.save.adventure.dimension != "overworld" {
            return None;
        }
        let generator = self.stream.world_gen();
        let index = generator
            .structures
            .sets()
            .iter()
            .position(|s| s.name.ends_with("strongholds"))?;
        let p = self.save.player;
        let chunks = generator
            .structures
            .placement(index)
            .ring_chunks(&generator.terrain, 128);
        let c = chunks.into_iter().min_by_key(|c| {
            let dx = c.x as i64 * 16 - p[0] as i64;
            let dz = c.z as i64 * 16 - p[2] as i64;
            dx * dx + dz * dz
        })?;
        let pos = [c.x * 16 + 8, 32, c.z * 16 + 8];
        self.save.adventure.stronghold = Some(pos);
        Some(pos)
    }
    /// Item use takes precedence over placing a block. No free resource grants.
    pub fn use_item(&mut self, target: Option<BlockPos>, place: Option<BlockPos>) -> bool {
        let Some(item) = self.inventory.slots[self.save.selected.min(8)]
            .as_ref()
            .map(|s| s.id.clone())
        else {
            return false;
        };
        match item.as_str() {
            "minecraft:flint_and_steel" => {
                if let Some(p) = place {
                    if self.ignite_portal(p) {
                        self.inventory.wear_tool(self.save.selected.min(8), 1);
                    } else if self.block_name(p).is_none() {
                        self.set_block(p, Some(Block::new("minecraft:fire")));
                        self.inventory.wear_tool(self.save.selected.min(8), 1);
                    }
                }
                true
            }
            "minecraft:ender_eye" => {
                if target.is_some_and(|p| self.insert_eye(p)) {
                    return true;
                }
                if let Some(p) = self.locate_stronghold() {
                    let dx = p[0] as f64 - self.save.player[0];
                    let dz = p[2] as f64 - self.save.player[2];
                    let bearing = (-dx).atan2(dz).to_degrees();
                    self.notices.push(format!("Eye of Ender: stronghold {:.0} blocks away · Minecraft bearing {:.0}° · X {} Z {}. Dig down near the destination.",dx.hypot(dz),bearing,p[0],p[2]));
                    let from = [
                        self.save.player[0],
                        self.save.player[1] + 1.6,
                        self.save.player[2],
                    ];
                    let dir = glam_mc::DVec3::new(dx, 0., dz).normalize_or_zero();
                    self.save.adventure.projectiles.push(Projectile {
                        position: from,
                        velocity: [dir.x * 4., 2., dir.z * 4.],
                        life: 3.,
                        kind: 2,
                    });
                    self.consume_selected();
                }
                true
            }
            "minecraft:bucket" => {
                if let Some(p) = target {
                    if let Some(name) = self.block_name(p).filter(|s| s == "water" || s == "lava") {
                        self.set_block(p, None);
                        self.consume_selected();
                        self.add_item(format!("minecraft:{name}_bucket"), 1);
                    }
                }
                true
            }
            "minecraft:water_bucket" | "minecraft:lava_bucket" => {
                if let Some(p) = place {
                    let liquid = if item.ends_with("water_bucket") {
                        "water"
                    } else {
                        "lava"
                    };
                    if liquid != "water" || self.save.adventure.dimension != "nether" {
                        if self.block_name(p).is_none() {
                            self.set_block(p, Some(Block::new(&format!("minecraft:{liquid}"))));
                            self.consume_selected();
                            self.add_item("minecraft:bucket".into(), 1);
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
    pub fn tick_adventure(&mut self, dt: f32, alive: bool) {
        self.portal_cooldown = (self.portal_cooldown - dt).max(0.);
        if !alive {
            self.portal_clock = 0.;
            return;
        }
        let p = block_pos(self.save.player);
        let block = self
            .block_name(p)
            .or_else(|| self.block_name((p.0, p.1 - 1, p.2)));
        if self.portal_cooldown == 0.
            && matches!(block.as_deref(), Some("nether_portal" | "end_portal"))
        {
            self.portal_clock += dt;
            if self.portal_clock
                >= if block.as_deref() == Some("end_portal") {
                    0.15
                } else {
                    4.
                }
            {
                let dimension = self.save.adventure.dimension.clone();
                if block.as_deref() == Some("end_portal")
                    && dimension == "end"
                    && self.save.adventure.defeated
                {
                    self.save.adventure.completed = true;
                    self.screen = survival::Screen::Victory;
                    self.portal_cooldown = 60.;
                    return;
                }
                let (dest, target) = if block.as_deref() == Some("end_portal") {
                    (Dimension::End, None)
                } else if dimension == "nether" {
                    (
                        Dimension::Overworld,
                        self.save
                            .adventure
                            .returns
                            .get("overworld")
                            .copied()
                            .or(Some([
                                self.save.player[0] * 8.,
                                70.,
                                self.save.player[2] * 8.,
                            ])),
                    )
                } else {
                    (
                        Dimension::Nether,
                        self.save.adventure.returns.get("nether").copied().or(Some([
                            self.save.player[0] / 8.,
                            64.,
                            self.save.player[2] / 8.,
                        ])),
                    )
                };
                if let Err(e) = self.change_dimension(dest, target) {
                    self.notices.push(e);
                    self.portal_cooldown = 10.;
                }
            }
        } else {
            self.portal_clock = 0.;
        }
        if self.save.adventure.dimension == "nether" {
            self.tick_blazes(dt);
        }
        if self.save.adventure.dimension == "end" {
            self.tick_dragon(dt);
        }
        self.tick_projectiles(dt);
        self.update_adventure_targets();
    }
    fn build_end_arena(&mut self) {
        // The generator already built the pillars, cages and fire for this seed.
        self.save.adventure.dragon.crystals =
            terrain::terrain::end_crystal_positions(self.save.seed)
                .into_iter()
                .map(|position| Crystal {
                    position,
                    alive: true,
                })
                .collect();
        self.save.adventure.arena_built = true;
        self.build_exit(false);
    }
    fn build_exit(&mut self, active: bool) {
        for x in -3..=3 {
            for z in -3..=3 {
                if x * x + z * z > 10 {
                    continue;
                }
                self.set_block((x, 63, z), Some(Block::new("minecraft:bedrock")));
                self.set_block(
                    (x, 64, z),
                    if active && x * x + z * z < 7 {
                        Some(Block::new("minecraft:end_portal"))
                    } else {
                        None
                    },
                );
            }
        }
        for y in 64..=67 {
            self.set_block((0, y, 0), Some(Block::new("minecraft:bedrock")));
        }
        if active {
            self.set_block((0, 68, 0), Some(Block::new("minecraft:dragon_egg")));
        }
    }
    fn damage_from(&mut self, damage: f32, p: [f64; 3], projectile: u8) {
        self.player_hits.push(crate::MobPlayerHit { attacker: if projectile==0 {"minecraft:blaze"}else{"minecraft:ender_dragon"}.into(), method: if projectile==3 {"dragon breath"}else{"fireball"}, hit: mc_entities::world::PlayerHit {
            player_id: 1,
            damage,
            kind: mc_entities::world::PlayerHitKind::Melee {
                attacker: glam_mc::DVec3::from_array(p),
                hunger_ticks: 0,
                lift: 0.,
            },
            source: None,
        }});
    }
    fn visible(&self, a: [f64; 3], b: [f64; 3]) -> bool {
        let a = to_source(self.save.origin, a);
        let b = to_source(self.save.origin, b);
        !self
            .collision
            .trace_box(a, b, Vec3::ZERO, Vec3::ZERO, contents::MASK_SHOT)
            .hit()
    }
    fn tick_blazes(&mut self, dt: f32) {
        if self.save.day_ticks as u64 % 100 == 0 && self.save.adventure.blazes.len() < 6 {
            let p = block_pos(self.save.player);
            let mut spawn = None;
            'search: for dx in (-24..=24).step_by(4) {
                for dz in (-24..=24).step_by(4) {
                    for dy in -12..=12 {
                        let at = (p.0 + dx, p.1 + dy, p.2 + dz);
                        if self.block_name(at).as_deref() == Some("nether_bricks")
                            && self.block_name((at.0, at.1 + 1, at.2)).is_none()
                            && dx * dx + dz * dz > 64
                        {
                            spawn = Some([at.0 as f64 + 0.5, at.1 as f64 + 2., at.2 as f64 + 0.5]);
                            break 'search;
                        }
                    }
                }
            }
            if let Some(p) = spawn {
                let id = self.save.adventure.next_id;
                self.save.adventure.next_id += 1;
                self.save.adventure.blazes.push(Blaze {
                    id,
                    position: p,
                    health: 20.,
                    cooldown: 3.,
                });
            }
        }
        let player = [
            self.save.player[0],
            self.save.player[1] + 1.,
            self.save.player[2],
        ];
        let mut hits = vec![];
        let mut blazes = std::mem::take(&mut self.save.adventure.blazes);
        for b in &mut blazes {
            let delta = glam_mc::DVec3::from_array(player) - glam_mc::DVec3::from_array(b.position);
            let dist = delta.length();
            if dist > 6. && dist < 32. {
                let next = (glam_mc::DVec3::from_array(b.position)
                    + delta.normalize_or_zero() * dt as f64 * 1.4)
                    .to_array();
                if self.visible(b.position, next) {
                    b.position = next;
                }
            }
            b.cooldown -= dt;
            if b.cooldown <= 0. && dist < 24. && self.visible(b.position, player) {
                hits.push(b.position);
                b.cooldown = 3.;
            }
        }
        self.save.adventure.blazes = blazes;
        for p in hits {
            self.launch_projectile(p, player, 0);
            self.sounds.push(("entity.blaze.shoot".into(), p));
        }
    }
    fn tick_dragon(&mut self, dt: f32) {
        if self.save.adventure.defeated {
            return;
        }
        let d = &mut self.save.adventure.dragon;
        if d.health <= 0. {
            d.death_time += dt;
            if d.death_time >= 10. {
                self.save.adventure.defeated = true;
                self.build_exit(true);
                self.notices.push("Free the End! The dragon is defeated. Enter the return portal to finish your run.".into());
            }
            return;
        }
        let old = d.phase_time;
        d.phase_time += dt;
        let t = d.phase_time;
        let cycle = t % 40.;
        let next = if cycle > 30. {
            [0., 68., 0.]
        } else {
            let a = t as f64 * 0.16;
            [a.cos() * 34., 78. + (a * 0.6).sin() * 7., a.sin() * 34.]
        };
        let pos = glam_mc::DVec3::from_array(d.position)
            .lerp(glam_mc::DVec3::from_array(next), (dt * 1.3) as f64);
        d.position = pos.to_array();
        if d.crystals
            .iter()
            .any(|c| c.alive && (glam_mc::DVec3::from_array(c.position) - pos).length() < 40.)
        {
            d.health = (d.health + dt * 2.).min(200.);
        }
        let attack = (t / 3.) as i32 != (old / 3.) as i32;
        let origin = d.position;
        if attack {
            let target = [
                self.save.player[0],
                self.save.player[1] + 1.,
                self.save.player[2],
            ];
            if self.visible(origin, target) {
                self.launch_projectile(origin, target, 1);
                self.sounds
                    .push(("entity.ender_dragon.growl".into(), origin));
            }
        }
    }
    pub fn hit_adventure(&mut self, key: u64, damage: f32) -> bool {
        if key < BOSS {
            return false;
        }
        if damage <= 0. || !damage.is_finite() { return true; }
        let id = key - BOSS;
        let mut hurt_at = None;
        if id == 0 && self.save.adventure.dimension == "end" {
            let d = &mut self.save.adventure.dragon;
            if d.health <= 0. { return true; }
            hurt_at = Some(d.position);
            let killed = d.health > 0. && d.health <= damage * 0.2;
            d.health = (d.health - damage * 0.2).max(0.);
            self.sounds
                .push(("entity.ender_dragon.hurt".into(), d.position));
            if killed { self.reward_mob("minecraft:ender_dragon"); }
        } else if (1..=10).contains(&id) {
            let mut killed = false;
            if let Some(c) = self.save.adventure.dragon.crystals.get_mut(id as usize - 1).filter(|c|c.alive) {
                c.alive = false;
                let pos = c.position;
                let d = &mut self.save.adventure.dragon;
                killed = d.health > 0. && d.health <= 10.;
                d.health = (d.health - 10.).max(0.);
                self.sounds.push(("entity.generic.explode".into(), pos));
            }
            if killed { self.reward_mob("minecraft:ender_dragon"); }
        } else if let Some(i) = self.save.adventure.blazes.iter().position(|b| b.id == id) {
            let b = &mut self.save.adventure.blazes[i];
            if b.health <= 0. { return true; }
            hurt_at = Some(b.position);
            b.health -= damage * 0.2;
            if b.health <= 0. {
                let b = self.save.adventure.blazes.remove(i);
                self.reward_mob("minecraft:blaze");
                self.drop_stack(ItemStack::new("minecraft:blaze_rod", 1), b.position);
                self.sounds.push(("entity.blaze.death".into(), b.position));
            }
        }
        if let Some(position) = hurt_at {
            self.adventure_hurt.insert(key, 0.5);
            self.mob_hits.push(MobHitFx { point:to_source(self.save.origin,position), from:to_source(self.save.origin,self.save.player) });
        }
        true
    }
    fn update_adventure_targets(&mut self) {
        let mut boxes = vec![];
        let d = &self.save.adventure.dragon;
        if self.save.adventure.dimension == "end" && !self.save.adventure.defeated {
            if d.health > 0. {
                for (position, radius) in self.dragon_model.bounds(d) {
                    boxes.push((BOSS, position, radius));
                }
            }
            for (i, c) in d.crystals.iter().enumerate() {
                if c.alive {
                    boxes.push((BOSS + 1 + i as u64, c.position, [1., 1.5, 1.]));
                }
            }
        }
        if self.save.adventure.dimension == "nether" {
            for b in &self.save.adventure.blazes {
                boxes.push((BOSS + b.id, b.position, [0.4, 0.9, 0.4]));
            }
        }
        let mut targets = self.collision.targets.write().unwrap();
        targets.retain(|(k, _, _)| *k < BOSS);
        for (id, p, r) in boxes {
            let a = to_source(self.save.origin, [p[0] - r[0], p[1] - r[1], p[2] - r[2]]);
            let b = to_source(self.save.origin, [p[0] + r[0], p[1] + r[1], p[2] + r[2]]);
            targets.push((id, a.min(b), a.max(b)));
        }
    }
}
impl Runtime {
    pub fn append_adventure_mesh(&self, mesh: &mut terrain::mesh::ChunkMesh, view: Option<&crate::rendering::EntityView>) {
        use glam_mc::{DVec3, Quat};
        use terrain::{cow_render::cube_tinted_pose as cube, pack::ResourceId};
        let t = self.save.day_ticks as f32 / 20.;
        if self.save.adventure.dimension == "nether" {
            let region = self
                .atlas
                .entity_region(&ResourceId::parse("minecraft:entity/blaze/blaze").unwrap());
            for b in &self.save.adventure.blazes {
                if view.is_some_and(|v|!v.visible(b.position,3.)) {continue;}
                let start = mesh.vertices.len();
                let p = DVec3::from_array(b.position) - DVec3::Y * 0.75;
                cube(
                    mesh,
                    p,
                    Quat::IDENTITY,
                    1.,
                    region,
                    0.,
                    15.,
                    [-4., -4., -4.],
                    [4., 4., 4.],
                    [0., 0.],
                    [0., 6., 0.],
                    Quat::from_rotation_y(t),
                    [1.; 3],
                    [64.; 2],
                    None,
                );
                for i in 0..12 {
                    let a =
                        t * (if i < 4 { 1. } else { -0.7 }) + i as f32 * std::f32::consts::TAU / 4.;
                    let radius = if i < 8 { 9. } else { 5. };
                    cube(
                        mesh,
                        p,
                        Quat::IDENTITY,
                        1.,
                        region,
                        0.,
                        15.,
                        [-1., 0., -1.],
                        [1., 8., 1.],
                        [0., 16.],
                        [
                            a.cos() * radius,
                            10. + (i / 4) as f32 * 5.,
                            a.sin() * radius,
                        ],
                        Quat::IDENTITY,
                        [1.; 3],
                        [64.; 2],
                        None,
                    );
                }
                if self.adventure_hurt.contains_key(&(BOSS+b.id)) {
                    for v in &mut mesh.vertices[start..] { v.color[3] = -terrain::client_mobs::RED_OVERLAY_ALPHA; }
                }
            }
        }
        for shot in &self.save.adventure.projectiles {
            if view.is_some_and(|v|!v.visible(shot.position,4.)) {continue;}
            let region = self.atlas.region(
                &ResourceId::parse(if shot.kind == 2 {
                    "minecraft:item/ender_eye"
                } else {
                    "minecraft:entity/enderdragon/dragon_fireball"
                })
                .unwrap(),
            );
            let p = DVec3::from_array(shot.position);
            let size = if shot.kind == 3 { 32. } else { 5. };
            cube(
                mesh,
                p,
                Quat::from_rotation_y(t),
                1.,
                region,
                0.,
                15.,
                [-size, -size, -size],
                [size, size, size],
                [0., 0.],
                [0., 24., 0.],
                Quat::from_rotation_x(t),
                if shot.kind == 0 {
                    [1., 0.45, 0.1]
                } else {
                    [1.; 3]
                },
                [32.; 2],
                Some([8.; 3]),
            );
        }
        if self.save.adventure.dimension != "end" {
            return;
        }
        let d = &self.save.adventure.dragon;
        let region = self
            .atlas
            .entity_region(&ResourceId::parse("minecraft:entity/end_crystal/end_crystal").unwrap());
        for c in &d.crystals {
            if c.alive && view.is_none_or(|v|v.visible(c.position,3.)) {
                let p = DVec3::from_array(c.position);
                for layer in 0..3 {
                    let size = 8. - layer as f32 * 2.;
                    cube(
                        mesh,
                        p,
                        Quat::from_rotation_y(t),
                        1.,
                        region,
                        0.,
                        15.,
                        [-size, -size, -size],
                        [size, size, size],
                        [if layer == 2 { 32. } else { 0. }, 0.],
                        [0., 10. + (t * 2.).sin() * 3., 0.],
                        Quat::from_rotation_z(0.785 + t * 0.4),
                        [1.; 3],
                        [64., 32.],
                        None,
                    );
                }
            }
        }
        if self.save.adventure.defeated {
            return;
        }
        let start = mesh.vertices.len();
        self.dragon_model.append(mesh, d, &self.atlas);
        if self.adventure_hurt.contains_key(&BOSS) {
            for v in &mut mesh.vertices[start..] { v.color[3] = -terrain::client_mobs::RED_OVERLAY_ALPHA; }
        }
    }
}
impl Runtime {
    fn launch_projectile(&mut self, from: [f64; 3], target: [f64; 3], kind: u8) {
        let velocity = (glam_mc::DVec3::from_array(target) - glam_mc::DVec3::from_array(from))
            .normalize_or_zero()
            * if kind == 0 { 10. } else { 14. };
        self.save.adventure.projectiles.push(Projectile {
            position: from,
            velocity: velocity.to_array(),
            life: 8.,
            kind,
        });
    }
    fn tick_projectiles(&mut self, dt: f32) {
        let mut next = vec![];
        let mut damage = vec![];
        let player = glam_mc::DVec3::from_array(self.save.player) + glam_mc::DVec3::Y;
        for mut shot in std::mem::take(&mut self.save.adventure.projectiles) {
            shot.life -= dt;
            let old = glam_mc::DVec3::from_array(shot.position);
            let p = old + glam_mc::DVec3::from_array(shot.velocity) * dt as f64;
            let hit = shot.kind < 2
                && (!self.visible(shot.position, p.to_array()) || (p - player).length() < 0.8);
            shot.position = p.to_array();
            if shot.kind == 3 {
                if (p - player).length() < 3. && ((shot.life / dt) as i32 % 32 == 0) {
                    damage.push((1., shot.position, shot.kind));
                }
            }
            if hit {
                if (p - player).length() < if shot.kind == 0 { 1.5 } else { 3. } {
                    damage.push((if shot.kind == 0 { 4. } else { 6. }, shot.position, shot.kind));
                }
                self.sounds
                    .push(("entity.generic.explode".into(), shot.position));
                if shot.kind == 1 {
                    next.push(Projectile {
                        position: shot.position,
                        velocity: [0.; 3],
                        life: 8.,
                        kind: 3,
                    });
                }
            } else if shot.life > 0. {
                next.push(shot);
            } else if shot.kind == 2 && self.save.day_ticks as u64 % 5 != 0 {
                self.drop_stack(ItemStack::new("minecraft:ender_eye", 1), shot.position);
            }
        }
        self.save.adventure.projectiles = next;
        for (d, p, kind) in damage {
            self.damage_from(d, p, kind);
        }
    }
}
