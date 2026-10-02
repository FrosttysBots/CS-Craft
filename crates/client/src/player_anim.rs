//! Third-person player animation, built from CS:GO's own animation set.
//!
//! A simplified version of the game's layered animation state:
//! * legs: the 9-way locomotion blend (`move_r`/`move_w`/`move_c`) over
//!   `move_yaw` (movement direction relative to facing), or jump/fall poses
//!   in the air;
//! * upper body (`spine_1` and its children): the weapon's aim pose
//!   (`rifle_aim_idle`, `pistol_aim_run`, ...) blended over look pitch;
//! * gestures on top: reload (full upper-body pose) and fire (additive).
//!
//! Clips are decoded on first use from the include model (`animset_t.mdl`)
//! and retargeted by bone name onto the character's skeleton.

use mdl::anim::Clip;
use mdl::skeleton::{self, Bone};
use std::collections::HashMap;
use std::sync::Arc;
use weapons::WeaponClass;

type Pose = Vec<(glam::Vec3, glam::Quat)>;

pub struct PlayerAnimSet {
    bones: Vec<Bone>,
    upper: Vec<bool>,
    source: Arc<mdl::Model>,
    clips: HashMap<String, Option<Arc<Clip>>>,
    grids: HashMap<String, (Arc<Vec<String>>, [i32; 2])>,
    menu_idle: Option<Clip>,
}

/// Inputs for one character this frame.
#[derive(Clone, Copy, Debug)]
pub struct AnimInput {
    pub speed: f32,
    /// Movement direction relative to facing, degrees (0 = forward).
    pub move_yaw: f32,
    pub on_ground: bool,
    pub ducked: bool,
    pub pitch: f32,
    pub class: WeaponClass,
    pub time_since_fire: f32,
    /// 0..1 while reloading.
    pub reload_progress: Option<f32>,
    pub air_time: f32,
}

#[derive(Clone, Debug, Default)]
pub struct AnimState {
    pub move_cycle: f32,
}

fn prefix(class: WeaponClass) -> &'static str {
    match class {
        WeaponClass::Knife => "knife",
        WeaponClass::Pistol => "pistol",
        WeaponClass::Sniper => "sniper",
        _ => "rifle",
    }
}

impl PlayerAnimSet {
    /// `character` provides the skeleton; animations come from its include
    /// models, loaded through `fs`.
    pub fn load(character: &mdl::Model, fs: &vfs::Vfs) -> Option<Self> {
        let include = character.include_models.first()?;
        let source = mdl::Model::load_header(&fs.read(include)?).ok()?;
        let bones = character.bones.clone();
        // Upper body: spine_1 and everything below it in the hierarchy.
        let root = character.bone_index("spine_1");
        let mut upper = vec![false; bones.len()];
        for (i, b) in bones.iter().enumerate() {
            upper[i] = Some(i) == root || (b.parent >= 0 && upper[b.parent as usize]);
        }
        let grids = source.sequences.iter().map(|s| {
            let names = s.anims.iter().map(|&i| source.animations.get(i as usize).map(|a| a.name.clone()).unwrap_or_default()).collect();
            (s.name.clone(), (Arc::new(names), s.group_size))
        }).collect();
        let mut set = Self {
            grids,
            bones,
            upper,
            source: Arc::new(source),
            clips: HashMap::new(),
            menu_idle: None,
        };
        if let Some(source)=fs.read("models/player/custom_player/uiplayer/animset_uiplayer_loadout.mdl")
            .and_then(|bytes|mdl::Model::load_header(&bytes).ok()) {
            if let Some(mut clip)=source.animation_index("@ct_loadout_rifle_idle_handrepo_m4")
                .and_then(|index|source.decode_animation(index,None).ok())
                .map(|clip|mdl::anim::retarget(&clip,&source.bones,&set.bones)) {
                clip.looping=true;
                set.menu_idle=Some(clip);
            }
        }
        // Decode the clips used by the current animation controller during
        // loading, so a first encounter/reload/jump cannot stall a frame.
        let mut names = std::collections::HashSet::new();
        for (sequence, (grid, _)) in &set.grids {
            if sequence.starts_with("move_") || ["rifle", "pistol", "knife", "sniper"].iter().any(|p| sequence.starts_with(&format!("{p}_aim_"))) {
                names.extend(grid.iter().cloned());
            }
        }
        for prefix in ["rifle", "pistol", "knife", "sniper"] {
            for action in ["fire", "fire_crouch", "reload", "reload_crouch"] { names.insert(format!("@{prefix}_{action}")); }
        }
        names.extend(["a_jump_onspot", "a_jump_onspot_crouch", "a_fall"].map(str::to_string));
        for name in names { set.clip(&name); }
        Some(set)
    }

    pub fn bones(&self) -> &[Bone] {
        &self.bones
    }

    /// Original Panorama loadout idle, using real time independently of a paused match.
    pub fn menu_pose(&self,time:f32)->Option<Vec<glam::Mat4>> {
        let clip=self.menu_idle.as_ref()?;
        let mut pose=skeleton::rest_pose(&self.bones);
        clip.sample(time,&mut pose);
        Some(skeleton::model_space(&self.bones,&pose))
    }

    /// A clip retargeted onto the character skeleton (cached).
    fn clip(&mut self, name: &str) -> Option<Arc<Clip>> {
        if let Some(c) = self.clips.get(name) {
            return c.clone();
        }
        let decoded = self.source.animation_index(name).and_then(|i| self.source.decode_animation(i, None).ok()).map(|clip| {
            let map: Vec<Option<usize>> = self
                .bones
                .iter()
                .map(|b| self.source.bone_index(&b.name))
                .collect();
            let frames = clip
                .frames
                .iter()
                .map(|f| {
                    self.bones
                        .iter()
                        .zip(&map)
                        .map(|(b, m)| m.and_then(|i| f.get(i).copied()).unwrap_or(if clip.delta { (glam::Vec3::ZERO, glam::Quat::IDENTITY) } else { (b.pos, b.quat) }))
                        .collect()
                })
                .collect();
            let animated = map.iter().map(|m| m.is_some_and(|i| clip.animated.get(i).copied().unwrap_or(false))).collect();
            Arc::new(Clip { frames, animated, ..clip })
        });
        self.clips.insert(name.to_string(), decoded.clone());
        decoded
    }

    /// Animation names of a sequence's blend grid.
    fn sequence_anims(&self, seq: &str) -> Option<(Arc<Vec<String>>, [i32; 2])> {
        self.grids.get(seq).cloned()
    }

    fn sample(&mut self, name: &str, t: f32, out: &mut Pose) -> bool {
        match self.clip(name) {
            Some(c) => {
                c.sample(t, out);
                true
            }
            None => false,
        }
    }

    fn sample_normalized(&mut self, name: &str, cycle: f32, out: &mut Pose) -> bool {
        match self.clip(name) {
            Some(c) => {
                c.sample(cycle * c.duration(), out);
                true
            }
            None => false,
        }
    }

    /// Aim pose for a state (`idle`, `walk`, `run`, `crouch_idle`,
    /// `crouch_moving`) blended over pitch along the grid's centre column.
    fn aim_pose(&mut self, class: WeaponClass, state: &str, pitch: f32) -> Option<Pose> {
        let (names, size) = self.sequence_anims(&format!("{}_aim_{state}", prefix(class)))?;
        let cols = size[0].max(1) as usize;
        let rows = size[1].max(1) as usize;
        let col = cols / 2;
        // Row 0 looks up (pitch -90), the last row down (pitch +90).
        let x = ((pitch + 90.0) / 180.0).clamp(0.0, 1.0) * (rows - 1) as f32;
        let r0 = x.floor() as usize;
        let r1 = (r0 + 1).min(rows - 1);
        let s = x - r0 as f32;
        let mut a = skeleton::rest_pose(&self.bones);
        let mut b = a.clone();
        self.sample(&names[r0 * cols + col], 0.0, &mut a);
        self.sample(&names[r1 * cols + col], 0.0, &mut b);
        Some(blend(&a, &b, s))
    }

    /// Locomotion blend over the 9-way `move_yaw` grid.
    fn move_pose(&mut self, seq: &str, move_yaw: f32, cycle: f32) -> Option<Pose> {
        let (names, size) = self.sequence_anims(seq)?;
        let n = (size[0].max(1) as usize).min(names.len());
        if n < 2 {
            return None;
        }
        // Grid spans -180..180; entry 0 is "N" (forward) at -180.
        let yaw = (move_yaw + 180.0).rem_euclid(360.0) - 180.0;
        let x = (yaw + 180.0) / 360.0 * (n - 1) as f32;
        let i0 = (x.floor() as usize).min(n - 1);
        let i1 = (i0 + 1).min(n - 1);
        let s = x - i0 as f32;
        let mut a = skeleton::rest_pose(&self.bones);
        let mut b = a.clone();
        self.sample_normalized(&names[i0], cycle, &mut a);
        self.sample_normalized(&names[i1], cycle, &mut b);
        Some(blend(&a, &b, s))
    }

    /// Model-space (Source) bone matrices for this frame.
    pub fn evaluate(&mut self, st: &mut AnimState, input: &AnimInput, dt: f32) -> Vec<glam::Mat4> {
        let moving = input.on_ground && input.speed > 12.0;
        let aim_state = match (input.ducked, moving) {
            (true, true) => "crouch_moving",
            (true, false) => "crouch_idle",
            (false, true) if input.speed > 150.0 => "run",
            (false, true) => "walk",
            (false, false) => "idle",
        };
        let upper = self
            .aim_pose(input.class, aim_state, input.pitch)
            .or_else(|| self.aim_pose(WeaponClass::Rifle, aim_state, input.pitch))
            .unwrap_or_else(|| skeleton::rest_pose(&self.bones));

        let knife = input.class == WeaponClass::Knife;
        let lower = if !input.on_ground {
            let mut p = upper.clone();
            let name = if input.air_time < 0.45 {
                if input.ducked { "a_jump_onspot_crouch" } else { "a_jump_onspot" }
            } else {
                "a_fall"
            };
            self.sample(name, input.air_time.min(1.0), &mut p);
            p
        } else if moving {
            let (seq, ref_speed) = match (input.ducked, input.speed > 150.0) {
                (true, _) => (if knife { "move_knife_c" } else { "move_c" }, 85.0),
                (false, true) => (if knife { "move_knife_r" } else { "move_r" }, 250.0),
                (false, false) => (if knife { "move_knife_w" } else { "move_w" }, 130.0),
            };
            // One cycle per ~2.2 s at the reference speed.
            st.move_cycle = (st.move_cycle + dt * (input.speed / ref_speed) / 2.2).fract();
            self.move_pose(seq, input.move_yaw, st.move_cycle).unwrap_or_else(|| upper.clone())
        } else {
            upper.clone()
        };

        let mut local: Pose = self
            .upper
            .iter()
            .enumerate()
            .map(|(i, &u)| if u { upper[i] } else { lower[i] })
            .collect();

        let p = prefix(input.class);
        if let Some(progress) = input.reload_progress {
            let mut r = local.clone();
            let name = if input.ducked { format!("@{p}_reload_crouch") } else { format!("@{p}_reload") };
            if self.sample_normalized(&name, progress, &mut r) || self.sample_normalized(&format!("@{p}_reload"), progress, &mut r) {
                for (i, slot) in local.iter_mut().enumerate() {
                    if self.upper[i] {
                        *slot = r[i];
                    }
                }
            }
        } else if input.time_since_fire < 0.6 {
            let name = if input.ducked { format!("@{p}_fire_crouch") } else { format!("@{p}_fire") };
            if let Some(clip) = self.clip(&name).or_else(|| self.clip(&format!("@{p}_fire"))) {
                if clip.delta {
                    let mut d = vec![(glam::Vec3::ZERO, glam::Quat::IDENTITY); self.bones.len()];
                    clip.sample(input.time_since_fire, &mut d);
                    for (i, slot) in local.iter_mut().enumerate() {
                        if self.upper[i] {
                            slot.0 += d[i].0;
                            slot.1 = (slot.1 * d[i].1).normalize();
                        }
                    }
                }
            }
        }
        skeleton::model_space(&self.bones, &local)
    }
}

fn blend(a: &Pose, b: &Pose, t: f32) -> Pose {
    a.iter().zip(b).map(|(x, y)| (x.0.lerp(y.0, t), x.1.slerp(y.1, t))).collect()
}
