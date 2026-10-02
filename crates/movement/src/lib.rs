//! Player movement in the style of CS:GO.
//!
//! Written from the publicly documented behaviour of Quake-lineage movement
//! and CS:GO's default `sv_*` values: ground friction with a stop speed,
//! wish-direction acceleration, air acceleration capped at a 30 u/s wish
//! speed (which is what makes air strafing work), 18-unit step-ups, the
//! stamina penalty on jumping and landing, crouch hull switching (including
//! crouch-jumping), and the bunny-hop speed cap.
//!
//! The code only depends on a [`TraceWorld`], so it runs identically on the
//! server, in client prediction and in headless tests.
//!
//! Units: inches, seconds, degrees. Axes: Source (X forward, Y left, Z up).

use collision::{contents, Trace};
use glam::Vec3;

pub mod buttons {
    pub const JUMP: u32 = 1 << 0;
    pub const DUCK: u32 = 1 << 1;
    /// Walk (shift): 52 % speed, silent footsteps.
    pub const WALK: u32 = 1 << 2;
    pub const ATTACK: u32 = 1 << 3;
    pub const ATTACK2: u32 = 1 << 4;
    pub const RELOAD: u32 = 1 << 5;
    pub const USE: u32 = 1 << 6;
}

/// Anything movement can trace against.
pub trait TraceWorld {
    fn voxel_scale(&self)->f32 {0.}
    fn climbable(&self,_origin:Vec3,_mins:Vec3,_maxs:Vec3)->collision::Climbable {collision::Climbable::None}
    fn trace_player(&self,start:Vec3,end:Vec3,mins:Vec3,maxs:Vec3,mask:i32,_drop_scaffold:bool)->Trace {self.trace_hull(start,end,mins,maxs,mask)}
    fn trace_hull(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3, mask: i32) -> Trace;
}

impl TraceWorld for collision::CollisionWorld {
    fn voxel_scale(&self)->f32 {self.dynamic.as_ref().map_or(0.,|d|d.voxel_scale())}
    fn climbable(&self,origin:Vec3,mins:Vec3,maxs:Vec3)->collision::Climbable {self.dynamic.as_ref().map_or(collision::Climbable::None,|d|d.climbable(origin,mins,maxs))}
    fn trace_player(&self,start:Vec3,end:Vec3,mins:Vec3,maxs:Vec3,mask:i32,drop_scaffold:bool)->Trace {
        if let Some(d)=self.dynamic.as_ref().filter(|d|d.voxel_scale()>0.) {d.trace_player(start,end,mins,maxs,mask,drop_scaffold)} else {self.trace_box(start,end,mins,maxs,mask)}
    }
    fn trace_hull(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3, mask: i32) -> Trace {
        self.trace_box(start, end, mins, maxs, mask)
    }
}

/// Tunables, named after the console variables they mirror. Defaults are
/// CS:GO's competitive values.
#[derive(Clone, Debug)]
pub struct MoveConfig {
    pub gravity: f32,
    pub friction: f32,
    pub stop_speed: f32,
    pub accelerate: f32,
    pub air_accelerate: f32,
    pub air_max_wishspeed: f32,
    pub max_speed: f32,
    pub max_velocity: f32,
    pub step_size: f32,
    pub jump_impulse: f32,
    pub stamina_max: f32,
    pub stamina_jump_cost: f32,
    pub stamina_land_cost: f32,
    pub stamina_recovery_rate: f32,
    pub walk_speed_modifier: f32,
    pub duck_speed_modifier: f32,
    /// Crouch transition rate (fraction of a full crouch per second).
    pub duck_speed: f32,
    pub enable_bunnyhopping: bool,
    pub auto_bunnyhopping: bool,
    pub accelerate_use_weapon_speed: bool,
}

impl Default for MoveConfig {
    fn default() -> Self {
        Self {
            gravity: 800.0,
            friction: 5.2,
            stop_speed: 80.0,
            accelerate: 5.5,
            air_accelerate: 12.0,
            air_max_wishspeed: 30.0,
            max_speed: 320.0,
            max_velocity: 3500.0,
            step_size: 18.0,
            // sqrt(2 * 800 * 57): a 57-unit jump.
            jump_impulse: 301.993_38,
            stamina_max: 80.0,
            stamina_jump_cost: 0.080,
            stamina_land_cost: 0.050,
            stamina_recovery_rate: 60.0,
            walk_speed_modifier: 0.52,
            duck_speed_modifier: 0.34,
            duck_speed: 8.0,
            enable_bunnyhopping: false,
            auto_bunnyhopping: false,
            accelerate_use_weapon_speed: true,
        }
    }
}

pub const STAND_MINS: Vec3 = Vec3::new(-16.0, -16.0, 0.0);
pub const STAND_MAXS: Vec3 = Vec3::new(16.0, 16.0, 72.0);
pub const DUCK_MINS: Vec3 = Vec3::new(-16.0, -16.0, 0.0);
pub const DUCK_MAXS: Vec3 = Vec3::new(16.0, 16.0, 54.0);
pub const STAND_VIEW: f32 = 64.0;
pub const DUCK_VIEW: f32 = 46.0;
/// Upward speed above which a player can never be standing on ground.
pub const NON_JUMP_VELOCITY: f32 = 140.0;
/// Minimum surface normal Z that counts as walkable ground.
pub const MIN_GROUND_NORMAL: f32 = 0.7;
const MASK: i32 = contents::MASK_PLAYERSOLID;

/// Per-tick movement input.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveInput {
    /// Pitch, yaw, roll in degrees.
    pub view_angles: Vec3,
    /// -450..450 (cl_forwardspeed).
    pub forward_move: f32,
    /// -450..450 (cl_sidespeed); positive is right.
    pub side_move: f32,
    pub buttons: u32,
    /// Max speed allowed by the held weapon (knife 250, AWP 200, ...).
    pub weapon_max_speed: f32,
}

/// Movement state of one player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerMove {
    /// Feet position.
    pub origin: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// Using the crouched hull.
    pub ducked: bool,
    /// 0 = standing, 1 = fully crouched (drives the eye height).
    pub duck_amount: f32,
    pub stamina: f32,
    /// Buttons held last tick (for press detection).
    pub old_buttons: u32,
    /// Speed at which the player last hit the ground (for landing effects).
    pub last_fall_speed: f32,
    /// Ticks since landing, saturating.
    pub ground_ticks: u32,
}

impl Default for PlayerMove {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            velocity: Vec3::ZERO,
            on_ground: false,
            ducked: false,
            duck_amount: 0.0,
            stamina: 0.0,
            old_buttons: 0,
            last_fall_speed: 0.0,
            ground_ticks: 0,
        }
    }
}

impl PlayerMove {
    pub fn spawn_at(origin: Vec3) -> Self {
        Self {
            origin,
            ..Default::default()
        }
    }

    pub fn hull(&self) -> (Vec3, Vec3) {
        if self.ducked { (DUCK_MINS, DUCK_MAXS) } else { (STAND_MINS, STAND_MAXS) }
    }

    /// Eye height above the feet.
    pub fn view_offset(&self) -> f32 {
        if self.ducked && !self.on_ground {
            // Crouch-jumping: the hull was pulled up, the eyes stay put.
            return DUCK_VIEW;
        }
        STAND_VIEW + (DUCK_VIEW - STAND_VIEW) * self.duck_amount
    }

    pub fn eye_position(&self) -> Vec3 {
        self.origin + Vec3::new(0.0, 0.0, self.view_offset())
    }

    pub fn speed_2d(&self) -> f32 {
        self.velocity.truncate().length()
    }
}

/// Events produced by one movement tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveEvents {
    pub jumped: bool,
    /// Landed this tick with the given downward speed.
    pub landed: Option<f32>,
}

/// Source `AngleVectors`: forward, right, up from pitch/yaw/roll.
pub fn angle_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
    let (sp, cp) = angles.x.to_radians().sin_cos();
    let (sy, cy) = angles.y.to_radians().sin_cos();
    let (sr, cr) = angles.z.to_radians().sin_cos();
    let forward = Vec3::new(cp * cy, cp * sy, -sp);
    let right = Vec3::new(-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp);
    let up = Vec3::new(cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp);
    (forward, right, up)
}

struct Mover<'a, W: TraceWorld + ?Sized> {
    pm: &'a mut PlayerMove,
    cfg: &'a MoveConfig,
    world: &'a W,
    dt: f32,
    max_speed: f32,
    drop_scaffold: bool,
}

impl<W: TraceWorld + ?Sized> Mover<'_, W> {
    fn trace(&self, start: Vec3, end: Vec3) -> Trace {
        let (mins, maxs) = self.pm.hull();
        self.world.trace_player(start, end, mins, maxs, MASK,self.drop_scaffold)
    }

    fn start_gravity(&mut self) {
        self.pm.velocity.z -= self.cfg.gravity * 0.5 * self.dt;
    }

    fn finish_gravity(&mut self) {
        self.pm.velocity.z -= self.cfg.gravity * 0.5 * self.dt;
    }

    fn check_velocity(&mut self) {
        let m = self.cfg.max_velocity;
        self.pm.velocity = self.pm.velocity.clamp(Vec3::splat(-m), Vec3::splat(m));
    }

    fn friction(&mut self) {
        let v = self.pm.velocity;
        let speed = v.length();
        if speed < 0.1 {
            return;
        }
        let mut drop = 0.0;
        if self.pm.on_ground {
            let control = speed.max(self.cfg.stop_speed);
            drop = control * self.cfg.friction * self.dt;
        }
        let new_speed = (speed - drop).max(0.0);
        if new_speed != speed {
            self.pm.velocity = v * (new_speed / speed);
        }
    }

    fn accelerate(&mut self, wish_dir: Vec3, wish_speed: f32, accel: f32, input: &MoveInput) {
        let current = self.pm.velocity.dot(wish_dir);
        let add = wish_speed - current;
        if add <= 0.0 {
            return;
        }
        // Walking and crouching accelerate as if at full weapon speed, so
        // reaching the slower cap takes the same time as running.
        let slowed = input.buttons & (buttons::WALK | buttons::DUCK) != 0 || self.pm.ducked;
        let base = if self.cfg.accelerate_use_weapon_speed && slowed {
            self.max_speed.max(wish_speed)
        } else {
            wish_speed
        };
        let accel_speed = (accel * self.dt * base).min(add);
        self.pm.velocity += wish_dir * accel_speed;
    }

    fn air_accelerate(&mut self, wish_dir: Vec3, wish_speed: f32, accel: f32) {
        let capped = wish_speed.min(self.cfg.air_max_wishspeed);
        let current = self.pm.velocity.dot(wish_dir);
        let add = capped - current;
        if add <= 0.0 {
            return;
        }
        let accel_speed = (accel * wish_speed * self.dt).min(add);
        self.pm.velocity += wish_dir * accel_speed;
    }

    fn wish(&self, input: &MoveInput) -> (Vec3, f32) {
        let (mut forward, mut right, _) = angle_vectors(input.view_angles);
        forward.z = 0.0;
        right.z = 0.0;
        forward = forward.normalize_or_zero();
        right = right.normalize_or_zero();
        let wish_vel = forward * input.forward_move + right * input.side_move;
        let mut wish_speed = wish_vel.length();
        let wish_dir = if wish_speed > 0.0 { wish_vel / wish_speed } else { Vec3::ZERO };
        let mut cap = self.max_speed;
        if self.pm.ducked || (self.pm.on_ground && self.pm.duck_amount > 0.5) {
            cap *= self.cfg.duck_speed_modifier;
        } else if input.buttons & buttons::WALK != 0 {
            cap *= self.cfg.walk_speed_modifier;
        }
        if wish_speed > cap {
            wish_speed = cap;
        }
        (wish_dir, wish_speed)
    }

    fn clip_velocity(v: Vec3, normal: Vec3, overbounce: f32) -> Vec3 {
        let backoff = v.dot(normal) * overbounce;
        let mut out = v - normal * backoff;
        let adjust = out.dot(normal);
        if adjust < 0.0 {
            out -= normal * adjust;
        }
        out
    }

    /// Slide move with up to four bumps; returns whether anything was hit.
    fn try_player_move(&mut self) -> bool {
        let primal = self.pm.velocity;
        let mut original = self.pm.velocity;
        let mut planes: [Vec3; 5] = [Vec3::ZERO; 5];
        let mut num_planes = 0usize;
        let mut time_left = self.dt;
        let mut blocked = false;

        for _ in 0..4 {
            if self.pm.velocity.length_squared() == 0.0 {
                break;
            }
            let end = self.pm.origin + self.pm.velocity * time_left;
            let tr = self.trace(self.pm.origin, end);
            if tr.all_solid {
                self.pm.velocity = Vec3::ZERO;
                return true;
            }
            if tr.fraction > 0.0 {
                self.pm.origin = tr.end;
                original = self.pm.velocity;
                num_planes = 0;
            }
            if tr.fraction == 1.0 {
                break;
            }
            blocked = true;
            time_left -= time_left * tr.fraction;
            if num_planes >= planes.len() {
                self.pm.velocity = Vec3::ZERO;
                break;
            }
            planes[num_planes] = tr.normal;
            num_planes += 1;

            if num_planes == 1 && !self.pm.on_ground {
                self.pm.velocity = Self::clip_velocity(original, planes[0], 1.0);
                original = self.pm.velocity;
            } else {
                let mut i = 0;
                while i < num_planes {
                    self.pm.velocity = Self::clip_velocity(original, planes[i], 1.0);
                    let ok = (0..num_planes).all(|j| j == i || self.pm.velocity.dot(planes[j]) >= 0.0);
                    if ok {
                        break;
                    }
                    i += 1;
                }
                if i == num_planes {
                    if num_planes != 2 {
                        self.pm.velocity = Vec3::ZERO;
                        break;
                    }
                    let dir = planes[0].cross(planes[1]).normalize_or_zero();
                    self.pm.velocity = dir * dir.dot(self.pm.velocity);
                }
                if self.pm.velocity.dot(primal) <= 0.0 {
                    self.pm.velocity = Vec3::ZERO;
                    break;
                }
            }
        }
        blocked
    }

    fn stay_on_ground(&mut self) {
        let up = self.trace(self.pm.origin, self.pm.origin + Vec3::new(0.0, 0.0, 2.0));
        let start = up.end;
        let end = self.pm.origin - Vec3::new(0.0, 0.0, self.cfg.step_size);
        let tr = self.trace(start, end);
        if tr.fraction > 0.0
            && tr.fraction < 1.0
            && !tr.start_solid
            && tr.normal.z >= MIN_GROUND_NORMAL
            && (self.pm.origin.z - tr.end.z).abs() > 0.5 * collision::DIST_EPSILON
        {
            self.pm.origin = tr.end;
        }
    }

    fn step_move(&mut self) {
        let start_origin = self.pm.origin;
        let start_vel = self.pm.velocity;

        // Plain slide.
        self.try_player_move();
        let down_origin = self.pm.origin;
        let down_vel = self.pm.velocity;

        // Up a step, slide, then back down.
        self.pm.origin = start_origin;
        self.pm.velocity = start_vel;
        let step = Vec3::new(0.0, 0.0, self.cfg.step_size + collision::DIST_EPSILON);
        let tr = self.trace(self.pm.origin, self.pm.origin + step);
        if !tr.start_solid && !tr.all_solid {
            self.pm.origin = tr.end;
        }
        self.try_player_move();
        let tr = self.trace(self.pm.origin, self.pm.origin - step);
        if !tr.start_solid && !tr.all_solid {
            self.pm.origin = tr.end;
        }
        // Stepped onto something too steep to stand on: keep the plain slide.
        if tr.fraction < 1.0 && tr.normal.z < MIN_GROUND_NORMAL {
            self.pm.origin = down_origin;
            self.pm.velocity = down_vel;
            return;
        }
        let up_origin = self.pm.origin;
        let down_dist = (down_origin - start_origin).truncate().length_squared();
        let up_dist = (up_origin - start_origin).truncate().length_squared();
        if down_dist > up_dist {
            self.pm.origin = down_origin;
            self.pm.velocity = down_vel;
        } else {
            self.pm.velocity.z = down_vel.z;
        }
    }

    fn walk_move(&mut self, input: &MoveInput) {
        // Stamina slows you down for a moment after jumps and landings.
        if self.pm.stamina > 0.0 {
            let ratio = ((self.cfg.stamina_max - self.pm.stamina) / self.cfg.stamina_max).clamp(0.0, 1.0);
            let ratio = ratio.powf(self.dt / (1.0 / 70.0));
            self.pm.velocity.x *= ratio;
            self.pm.velocity.y *= ratio;
        }

        let (wish_dir, wish_speed) = self.wish(input);
        self.pm.velocity.z = 0.0;
        self.accelerate(wish_dir, wish_speed, self.cfg.accelerate, input);
        self.pm.velocity.z = 0.0;

        let speed = self.pm.velocity.length();
        if speed > self.max_speed {
            self.pm.velocity *= self.max_speed / speed;
        }
        if self.pm.velocity.length() < 1.0 {
            self.pm.velocity = Vec3::ZERO;
            return;
        }

        if self.world.voxel_scale()>0. && input.buttons & (buttons::DUCK|buttons::WALK)!=0 {
            self.guard_edge();
        }
        let dest = self.pm.origin + self.pm.velocity * self.dt;
        let tr = self.trace(self.pm.origin, dest);
        if tr.fraction == 1.0 {
            self.pm.origin = tr.end;
            self.stay_on_ground();
            return;
        }
        self.step_move();
        self.stay_on_ground();
    }

    fn guard_edge(&mut self) {
        let scale=self.world.voxel_scale();
        let step=scale*0.05;
        let supports=|x:f32,y:f32| {
            let p=self.pm.origin+Vec3::new(x,y,0.);
            let hit=self.trace(p,p-Vec3::Z*scale*0.6);
            hit.fraction<1. && hit.normal.z>=0.7
        };
        let reduce=|v:f32| if v.abs()<=step {0.} else {v-v.signum()*step};
        let mut x=self.pm.velocity.x*self.dt;let mut y=self.pm.velocity.y*self.dt;
        while x!=0. && !supports(x,0.) {x=reduce(x);}
        while y!=0. && !supports(0.,y) {y=reduce(y);}
        while x!=0. && y!=0. && !supports(x,y) {x=reduce(x);y=reduce(y);}
        self.pm.velocity.x=x/self.dt;self.pm.velocity.y=y/self.dt;
    }
    fn climb_move(&mut self,input:&MoveInput,kind:collision::Climbable) {
        let scale=self.world.voxel_scale();
        self.pm.on_ground=false;
        self.pm.velocity*=0.75;
        let (dir,speed)=self.wish(input);
        self.accelerate(dir,speed.min(scale*3.),self.cfg.accelerate,input);
        self.pm.velocity.x=self.pm.velocity.x.clamp(-scale*3.,scale*3.);
        self.pm.velocity.y=self.pm.velocity.y.clamp(-scale*3.,scale*3.);
        let horizontal=self.trace(self.pm.origin,self.pm.origin+Vec3::new(self.pm.velocity.x,self.pm.velocity.y,0.)*self.dt);
        let sneak=input.buttons&(buttons::DUCK|buttons::WALK)!=0;
        self.pm.velocity.z=if kind==collision::Climbable::Scaffolding && sneak {-scale*3.}
            else if sneak {0.}
            else if input.buttons&buttons::JUMP!=0 || (kind==collision::Climbable::Ladder && horizontal.fraction<1. && horizontal.normal.z.abs()<0.5) {scale*4.}
            else {self.pm.velocity.z.max(-scale*3.)-self.cfg.gravity*self.dt};
        self.pm.velocity.z=self.pm.velocity.z.max(-scale*3.);
        self.try_player_move();
    }
    fn air_move(&mut self, input: &MoveInput) {
        let (wish_dir, wish_speed) = self.wish(input);
        self.air_accelerate(wish_dir, wish_speed, self.cfg.air_accelerate);
        self.try_player_move();
    }

    fn check_jump(&mut self, input: &MoveInput, events: &mut MoveEvents) {
        if !self.pm.on_ground {
            return;
        }
        if !self.cfg.auto_bunnyhopping && self.pm.old_buttons & buttons::JUMP != 0 {
            return; // must re-press
        }
        if !self.cfg.enable_bunnyhopping {
            // Cap horizontal speed at 110 % of max on take-off.
            let max = self.max_speed * 1.1;
            let speed = self.pm.velocity.length();
            if speed > max && speed > 0.0 {
                self.pm.velocity *= max / speed;
            }
        }
        self.pm.on_ground = false;
        let mut impulse = self.cfg.jump_impulse;
        if self.pm.stamina > 0.0 {
            impulse *= ((self.cfg.stamina_max - self.pm.stamina) / self.cfg.stamina_max).clamp(0.0, 1.0);
        }
        if self.pm.ducked || input.buttons & buttons::DUCK != 0 && self.pm.duck_amount > 0.0 {
            self.pm.velocity.z = impulse;
        } else {
            self.pm.velocity.z += impulse;
        }
        self.pm.stamina = (self.cfg.stamina_jump_cost * self.cfg.jump_impulse).min(self.cfg.stamina_max);
        events.jumped = true;
    }

    fn categorize(&mut self, events: &mut MoveEvents) {
        let was_on_ground = self.pm.on_ground;
        let fall_speed = -self.pm.velocity.z;
        if self.pm.velocity.z > NON_JUMP_VELOCITY {
            self.pm.on_ground = false;
            return;
        }
        let point = self.pm.origin - Vec3::new(0.0, 0.0, 2.0);
        let tr = self.trace(self.pm.origin, point);
        if tr.fraction == 1.0 || tr.normal.z < MIN_GROUND_NORMAL || tr.start_solid && tr.all_solid {
            self.pm.on_ground = false;
            return;
        }
        self.pm.on_ground = true;
        // Snap down onto the ground when close.
        if !tr.start_solid {
            self.pm.origin = tr.end;
        }
        if !was_on_ground {
            events.landed = Some(fall_speed.max(0.0));
            self.pm.last_fall_speed = fall_speed.max(0.0);
            if fall_speed > 0.0 {
                self.pm.stamina = (self.cfg.stamina_land_cost * fall_speed).min(self.cfg.stamina_max);
            }
        }
    }

    fn duck(&mut self, input: &MoveInput) {
        let want_duck = input.buttons & buttons::DUCK != 0;
        let raise = STAND_MAXS.z - DUCK_MAXS.z;
        if want_duck {
            if !self.pm.ducked {
                if !self.pm.on_ground {
                    // Crouch in the air: pull the feet up, eyes stay level.
                    self.pm.ducked = true;
                    self.pm.duck_amount = 1.0;
                    let lifted = self.pm.origin + Vec3::new(0.0, 0.0, raise);
                    let tr = self.world.trace_hull(lifted, lifted, DUCK_MINS, DUCK_MAXS, MASK);
                    if !tr.start_solid {
                        self.pm.origin = lifted;
                    }
                } else {
                    self.pm.duck_amount = (self.pm.duck_amount + self.dt * self.cfg.duck_speed).min(1.0);
                    if self.pm.duck_amount >= 1.0 {
                        self.pm.ducked = true;
                    }
                }
            }
        } else {
            if self.pm.ducked {
                let target = if self.pm.on_ground {
                    self.pm.origin
                } else {
                    self.pm.origin - Vec3::new(0.0, 0.0, raise)
                };
                let tr = self.world.trace_hull(target, target, STAND_MINS, STAND_MAXS, MASK);
                if !tr.start_solid {
                    self.pm.ducked = false;
                    self.pm.origin = target;
                    if !self.pm.on_ground {
                        self.pm.duck_amount = 0.0;
                    }
                }
            }
            if !self.pm.ducked {
                self.pm.duck_amount = (self.pm.duck_amount - self.dt * self.cfg.duck_speed).max(0.0);
            }
        }
    }
}

/// Advances one player's movement by `dt` seconds.
pub fn player_move<W: TraceWorld + ?Sized>(
    pm: &mut PlayerMove,
    input: &MoveInput,
    cfg: &MoveConfig,
    world: &W,
    dt: f32,
) -> MoveEvents {
    let max_speed = if input.weapon_max_speed > 0.0 {
        input.weapon_max_speed.min(cfg.max_speed)
    } else {
        250.0f32.min(cfg.max_speed)
    };
    let mut events = MoveEvents::default();
    let mut m = Mover {
        pm,
        cfg,
        world,
        dt,
        max_speed,
        drop_scaffold:input.buttons&(buttons::DUCK|buttons::WALK)!=0,
    };

    m.pm.stamina = (m.pm.stamina - cfg.stamina_recovery_rate * dt).max(0.0);
    m.duck(input);
    let (mins,maxs)=m.pm.hull();
    let climb=world.climbable(m.pm.origin,mins,maxs);
    if climb!=collision::Climbable::None {
        m.climb_move(input,climb);
        m.categorize(&mut events);
        m.pm.old_buttons=input.buttons;
        // Catching a ladder arrests the fall, not a hard landing.
        events.landed=None;
        return events;
    }
    m.start_gravity();
    if input.buttons & buttons::JUMP != 0 {
        m.check_jump(input, &mut events);
    }
    if m.pm.on_ground {
        m.pm.velocity.z = 0.0;
        m.friction();
    }
    m.check_velocity();
    if m.pm.on_ground {
        m.walk_move(input);
    } else {
        m.air_move(input);
    }
    m.categorize(&mut events);
    m.check_velocity();
    m.finish_gravity();
    if m.pm.on_ground {
        m.pm.velocity.z = 0.0;
        m.pm.ground_ticks = m.pm.ground_ticks.saturating_add(1);
    } else {
        m.pm.ground_ticks = 0;
    }
    m.pm.old_buttons = input.buttons;
    events
}

#[cfg(test)]
mod tests;
