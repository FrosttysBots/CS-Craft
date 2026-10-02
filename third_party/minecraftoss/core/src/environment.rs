//! Environment attributes (26.3 `EnvironmentAttributeSystem`): sky, fog and
//! light colors, sun and moon angles and gameplay values, layered from the
//! dimension type's constants, biome attributes (blended around the camera
//! with `GaussianSampler`), timelines keyed to world clocks, and weather.
//!
//! Source-informed from the pinned 26.3 `net.minecraft.world.attribute`,
//! `world.timeline` and `util.KeyframeTrack*`/`EasingType`/`ARGB` classes.
//! Values that are not numbers, colors or flags (music, particles, spawn
//! settings) are carried as JSON and only overridden.

use serde_json::Value as Json;
use std::collections::HashMap;

/// How an attribute's values interpolate (`AttributeType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Float,
    /// `angle_degrees`: floats that partial-tick lerp the short way round.
    Angle,
    Rgb,
    Argb,
    Integer,
    Boolean,
    /// Enums (`moon_phase`, `tri_state`, ...): stepped, never blended.
    Text,
    /// Anything else, stepped and only overridden.
    Other,
}

/// An attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Float(f32),
    Rgb([f32; 3]),
    Argb([f32; 4]),
    Int(i32),
    Bool(bool),
    Text(String),
    Json(Json),
}

impl Value {
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Float(v) => *v,
            Self::Int(v) => *v as f32,
            _ => 0.0,
        }
    }

    pub fn as_rgb(&self) -> [f32; 3] {
        match self {
            Self::Rgb(c) => *c,
            Self::Argb([r, g, b, _]) => [*r, *g, *b],
            _ => [0.0; 3],
        }
    }

    pub fn as_argb(&self) -> [f32; 4] {
        match self {
            Self::Argb(c) => *c,
            Self::Rgb([r, g, b]) => [*r, *g, *b, 1.0],
            _ => [0.0; 4],
        }
    }

    pub fn as_bool(&self) -> bool {
        matches!(self, Self::Bool(true))
    }

    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(s) => s,
            _ => "",
        }
    }
}

/// One registered attribute (`EnvironmentAttributes`).
#[derive(Clone, Debug)]
pub struct Attribute {
    pub name: &'static str,
    pub kind: Kind,
    pub default: Value,
    range: Option<(f32, f32)>,
    spatial: bool,
    full_resolution_biomes: bool,
}

fn rgb24(color: u32) -> [f32; 3] {
    [((color >> 16) & 0xFF) as f32 / 255.0, ((color >> 8) & 0xFF) as f32 / 255.0, (color & 0xFF) as f32 / 255.0]
}

fn argb32(color: u32) -> [f32; 4] {
    let [r, g, b] = rgb24(color);
    [r, g, b, ((color >> 24) & 0xFF) as f32 / 255.0]
}

/// Every 26.3 attribute in registration order.
pub fn attributes() -> Vec<Attribute> {
    use Kind::*;
    let a = |name: &'static str, kind: Kind, default: Value, range: Option<(f32, f32)>, spatial: bool| Attribute { name, kind, default, range, spatial, full_resolution_biomes: false };
    let unit = Some((0.0, 1.0));
    let non_negative = Some((0.0, f32::INFINITY));
    let other = |name: &'static str| a(name, Other, Value::Json(Json::Null), None, false);
    let mut list = vec![
        a("visual/fog_color", Rgb, Value::Rgb(rgb24(0)), None, true),
        a("visual/fog_start_distance", Float, Value::Float(0.0), None, true),
        a("visual/fog_end_distance", Float, Value::Float(1024.0), non_negative, true),
        a("visual/sky_fog_end_distance", Float, Value::Float(512.0), non_negative, true),
        a("visual/cloud_fog_end_distance", Float, Value::Float(2048.0), non_negative, true),
        a("visual/water_fog_color", Rgb, Value::Rgb(rgb24(-16448205i32 as u32)), None, true),
        a("visual/water_fog_start_distance", Float, Value::Float(-8.0), None, true),
        a("visual/water_fog_end_distance", Float, Value::Float(96.0), non_negative, true),
        a("visual/sky_color", Rgb, Value::Rgb(rgb24(0)), None, true),
        a("visual/sunrise_sunset_color", Argb, Value::Argb(argb32(0)), None, true),
        a("visual/cloud_color", Argb, Value::Argb(argb32(0)), None, true),
        a("visual/cloud_height", Float, Value::Float(192.33), None, true),
        a("visual/sun_angle", Angle, Value::Float(0.0), None, true),
        a("visual/moon_angle", Angle, Value::Float(0.0), None, true),
        a("visual/star_angle", Angle, Value::Float(0.0), None, true),
        a("visual/moon_phase", Text, Value::Text("full_moon".into()), None, false),
        a("visual/star_brightness", Float, Value::Float(0.0), unit, true),
        a("visual/block_light_tint", Rgb, Value::Rgb(rgb24(-10100i32 as u32)), None, true),
        a("visual/sky_light_color", Rgb, Value::Rgb(rgb24(-1i32 as u32)), None, true),
        a("visual/sky_light_factor", Float, Value::Float(1.0), unit, true),
        a("visual/night_vision_color", Rgb, Value::Rgb(rgb24(-6710887i32 as u32)), None, true),
        a("visual/ambient_light_color", Rgb, Value::Rgb(rgb24(-16777216i32 as u32)), None, true),
        other("visual/default_dripstone_particle"),
        other("visual/ambient_particles"),
        other("audio/background_music"),
        a("audio/music_volume", Float, Value::Float(1.0), unit, false),
        other("audio/ambient_sounds"),
        a("audio/firefly_bush_sounds", Boolean, Value::Bool(false), None, false),
        a("gameplay/sky_light_level", Float, Value::Float(15.0), Some((0.0, 15.0)), false),
        a("gameplay/can_start_raid", Boolean, Value::Bool(true), None, false),
        a("gameplay/water_evaporates", Boolean, Value::Bool(false), None, false),
        other("gameplay/bed_rule"),
        other("gameplay/straw_bed_rule"),
        a("gameplay/respawn_anchor_works", Boolean, Value::Bool(false), None, false),
        a("gameplay/nether_portal_spawns_piglin", Boolean, Value::Bool(false), None, false),
        a("gameplay/fast_lava", Boolean, Value::Bool(false), None, false),
        a("gameplay/increased_fire_burnout", Boolean, Value::Bool(false), None, false),
        a("gameplay/eyeblossom_open", Text, Value::Text("default".into()), None, false),
        a("gameplay/turtle_egg_hatch_chance", Float, Value::Float(0.002), unit, false),
        a("gameplay/piglins_zombify", Boolean, Value::Bool(true), None, false),
        a("gameplay/snow_golem_melts", Boolean, Value::Bool(false), None, false),
        a("gameplay/creaking_active", Boolean, Value::Bool(false), None, false),
        a("gameplay/surface_slime_spawn_chance", Float, Value::Float(0.0), unit, false),
        a("gameplay/cat_waking_up_gift_chance", Float, Value::Float(0.0), unit, false),
        a("gameplay/bees_stay_in_hive", Boolean, Value::Bool(false), None, false),
        a("gameplay/monsters_burn", Boolean, Value::Bool(false), None, false),
        a("gameplay/can_pillager_patrol_spawn", Boolean, Value::Bool(true), None, false),
        other("gameplay/natural_mob_spawns"),
        a("gameplay/creature_world_gen_spawn_probability", Float, Value::Float(0.1), Some((0.0, 0.9999999)), false),
        a("gameplay/villager_activity", Text, Value::Text("minecraft:idle".into()), None, false),
        a("gameplay/baby_villager_activity", Text, Value::Text("minecraft:idle".into()), None, false),
    ];
    if let Some(spawns) = list.iter_mut().find(|a| a.name == "gameplay/natural_mob_spawns") {
        spawns.full_resolution_biomes = true;
    }
    list
}

/// `Mth.lerp` for floats.
fn lerp(alpha: f32, from: f32, to: f32) -> f32 {
    from + alpha * (to - from)
}

/// JOML `Vector*f.lerp` (`ARGB.srgbLerp`): `Math.fma`, which without
/// `joml.useMathFma` (off by default) rounds the multiply and the add.
fn joml_lerp<const N: usize>(alpha: f32, from: [f32; N], to: [f32; N]) -> [f32; N] {
    std::array::from_fn(|i| (to[i] - from[i]) * alpha + from[i])
}

/// `Mth.wrapDegrees`.
fn wrap_degrees(angle: f32) -> f32 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

/// Which of a type's lerp functions to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Keyframe,
    StateChange,
    Spatial,
    PartialTick,
}

/// `AttributeType.*Lerp` for one purpose.
pub fn lerp_value(kind: Kind, purpose: Purpose, alpha: f32, from: &Value, to: &Value) -> Value {
    match (kind, from, to) {
        (Kind::Angle, Value::Float(a), Value::Float(b)) if purpose == Purpose::PartialTick => {
            // `LerpFunction.ofDegrees(90)`.
            let delta = wrap_degrees(b - a);
            Value::Float(if delta.abs() >= 90.0 { *b } else { a + alpha * delta })
        }
        (Kind::Float | Kind::Angle, Value::Float(a), Value::Float(b)) => Value::Float(lerp(alpha, *a, *b)),
        (Kind::Rgb, Value::Rgb(a), Value::Rgb(b)) => Value::Rgb(joml_lerp(alpha, *a, *b)),
        (Kind::Argb, Value::Argb(a), Value::Argb(b)) => Value::Argb(joml_lerp(alpha, *a, *b)),
        (Kind::Integer, Value::Int(a), Value::Int(b)) => Value::Int(a + (alpha * (b - a) as f32).floor() as i32),
        _ => {
            // Stepped: keyframes switch at 1, state changes at 0, space at 0.5.
            let threshold = match purpose {
                Purpose::Keyframe => 1.0,
                Purpose::StateChange | Purpose::PartialTick => 0.0,
                Purpose::Spatial => 0.5,
            };
            if alpha >= threshold { to.clone() } else { from.clone() }
        }
    }
}

/// `AttributeModifier.OperationId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Override,
    AlphaBlend,
    Add,
    Subtract,
    Multiply,
    BlendToGray,
    Minimum,
    Maximum,
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
    Append,
    Overlay,
}

impl Op {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "override" => Self::Override,
            "alpha_blend" => Self::AlphaBlend,
            "add" => Self::Add,
            "subtract" => Self::Subtract,
            "multiply" => Self::Multiply,
            "blend_to_gray" => Self::BlendToGray,
            "minimum" => Self::Minimum,
            "maximum" => Self::Maximum,
            "and" => Self::And,
            "nand" => Self::Nand,
            "or" => Self::Or,
            "nor" => Self::Nor,
            "xor" => Self::Xor,
            "xnor" => Self::Xnor,
            "append" => Self::Append,
            "overlay" => Self::Overlay,
            _ => return None,
        })
    }
}

/// A modifier's argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Value(Value),
    /// `FloatWithAlpha`.
    FloatAlpha(f32, f32),
    /// `ColorModifier.BlendToGray`: brightness and factor.
    Gray(f32, f32),
}

impl Arg {
    fn lerp(&self, kind: Kind, op: Op, alpha: f32, to: &Arg) -> Arg {
        match (self, to) {
            (Arg::FloatAlpha(v0, a0), Arg::FloatAlpha(v1, a1)) => Arg::FloatAlpha(lerp(alpha, *v0, *v1), lerp(alpha, *a0, *a1)),
            (Arg::Gray(b0, f0), Arg::Gray(b1, f1)) => Arg::Gray(lerp(alpha, *b0, *b1), lerp(alpha, *f0, *f1)),
            (Arg::Value(a), Arg::Value(b)) => {
                // Argument lerps: the type's keyframe lerp for overrides,
                // float lerps for float ops, color lerps for color ops,
                // constant for boolean ops.
                let arg_kind = match (kind, op) {
                    (_, Op::Override) => kind,
                    (Kind::Boolean, _) => Kind::Boolean,
                    (_, _) => match b {
                        Value::Float(_) => Kind::Float,
                        Value::Rgb(_) => Kind::Rgb,
                        Value::Argb(_) => Kind::Argb,
                        Value::Int(_) => Kind::Integer,
                        _ => Kind::Other,
                    },
                };
                Arg::Value(lerp_value(arg_kind, Purpose::Keyframe, alpha, a, b))
            }
            _ => to.clone(),
        }
    }
}

/// One attribute's modifier and argument (`EnvironmentAttributeMap.Entry`).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub op: Op,
    pub arg: Arg,
}

/// `ARGB.alphaBlend(Vector4fc, Vector4fc)`.
fn alpha_blend4(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let (da, sa) = (dst[3], src[3]);
    if sa == 1.0 {
        return src;
    }
    if sa == 0.0 {
        return dst;
    }
    let alpha = sa + da * (1.0 - sa);
    let ch = |d: f32, s: f32| (s * sa + d * (alpha - sa)) / alpha;
    [ch(dst[0], src[0]), ch(dst[1], src[1]), ch(dst[2], src[2]), alpha]
}

fn greyscale(c: [f32; 3]) -> f32 {
    c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11
}

/// Applies an entry to a value (`AttributeModifier.apply`).
pub fn apply(kind: Kind, value: &Value, entry: &Entry) -> Value {
    match (entry.op, &entry.arg, value) {
        (Op::Override, Arg::Value(v), _) => v.clone(),
        (Op::AlphaBlend, Arg::FloatAlpha(v, a), Value::Float(x)) => Value::Float(lerp(*a, *x, *v)),
        (Op::Add, Arg::Value(Value::Float(v)), Value::Float(x)) => Value::Float(x + v),
        (Op::Subtract, Arg::Value(Value::Float(v)), Value::Float(x)) => Value::Float(x - v),
        (Op::Multiply, Arg::Value(Value::Float(v)), Value::Float(x)) => Value::Float(x * v),
        (Op::Minimum, Arg::Value(Value::Float(v)), Value::Float(x)) => Value::Float(x.min(*v)),
        (Op::Maximum, Arg::Value(Value::Float(v)), Value::Float(x)) => Value::Float(x.max(*v)),
        (Op::Add, Arg::Value(Value::Int(v)), Value::Int(x)) => Value::Int(x + v),
        (Op::Subtract, Arg::Value(Value::Int(v)), Value::Int(x)) => Value::Int(x - v),
        (Op::Multiply, Arg::Value(Value::Int(v)), Value::Int(x)) => Value::Int(x * v),
        (Op::Minimum, Arg::Value(Value::Int(v)), Value::Int(x)) => Value::Int(*x.min(v)),
        (Op::Maximum, Arg::Value(Value::Int(v)), Value::Int(x)) => Value::Int(*x.max(v)),
        (Op::AlphaBlend, Arg::Value(Value::Argb(src)), Value::Rgb(dst)) => {
            // `ARGB.alphaBlend(Vector3fc, Vector4fc)`: a JOML lerp by the source alpha.
            if src[3] == 0.0 {
                value.clone()
            } else {
                Value::Rgb(joml_lerp(src[3], *dst, [src[0], src[1], src[2]]))
            }
        }
        (Op::AlphaBlend, Arg::Value(Value::Argb(src)), Value::Argb(dst)) => Value::Argb(alpha_blend4(*dst, *src)),
        (Op::Add, Arg::Value(Value::Rgb(v)), Value::Rgb(x)) => Value::Rgb(std::array::from_fn(|i| (x[i] + v[i]).min(1.0))),
        (Op::Add, Arg::Value(Value::Rgb(v)), Value::Argb(x)) => Value::Argb([(x[0] + v[0]).min(1.0), (x[1] + v[1]).min(1.0), (x[2] + v[2]).min(1.0), x[3]]),
        (Op::Subtract, Arg::Value(Value::Rgb(v)), Value::Rgb(x)) => Value::Rgb(std::array::from_fn(|i| (x[i] - v[i]).max(0.0))),
        (Op::Subtract, Arg::Value(Value::Rgb(v)), Value::Argb(x)) => Value::Argb([(x[0] - v[0]).max(0.0), (x[1] - v[1]).max(0.0), (x[2] - v[2]).max(0.0), x[3]]),
        (Op::Multiply, Arg::Value(Value::Rgb(v)), Value::Rgb(x)) => {
            // `ARGB.multiply` returns an operand unchanged when the other is white.
            if *x == [1.0; 3] {
                Value::Rgb(*v)
            } else if *v == [1.0; 3] {
                value.clone()
            } else {
                Value::Rgb(std::array::from_fn(|i| x[i] * v[i]))
            }
        }
        (Op::Multiply, Arg::Value(Value::Argb(v)), Value::Argb(x)) => {
            if *x == [1.0; 4] {
                Value::Argb(*v)
            } else if *v == [1.0; 4] {
                value.clone()
            } else {
                Value::Argb(std::array::from_fn(|i| x[i] * v[i]))
            }
        }
        (Op::BlendToGray, Arg::Gray(brightness, factor), Value::Rgb(x)) => {
            let g = (greyscale(*x) * brightness).clamp(0.0, 1.0);
            Value::Rgb(joml_lerp(*factor, *x, [g; 3]))
        }
        (Op::BlendToGray, Arg::Gray(brightness, factor), Value::Argb(x)) => {
            let g = (greyscale([x[0], x[1], x[2]]) * brightness).clamp(0.0, 1.0);
            Value::Argb(joml_lerp(*factor, *x, [g, g, g, x[3]]))
        }
        (op, Arg::Value(Value::Bool(v)), Value::Bool(x)) => Value::Bool(match op {
            Op::And => *v && *x,
            Op::Nand => !*v || !*x,
            Op::Or => *v || *x,
            Op::Nor => !*v && !*x,
            Op::Xor => v ^ x,
            Op::Xnor => v == x,
            _ => *x,
        }),
        _ => {
            let _ = kind;
            value.clone()
        }
    }
}

/// `STRING_RGB_VEC3_COLOR` / `STRING_ARGB_VEC4_COLOR`: `#rrggbb`, `#aarrggbb` or a float list.
fn parse_color(json: &Json, alpha: bool) -> Option<Value> {
    if let Some(text) = json.as_str() {
        let hex = u32::from_str_radix(text.trim_start_matches('#'), 16).ok()?;
        return Some(if alpha { Value::Argb(argb32(hex)) } else { Value::Rgb(rgb24(hex)) });
    }
    let list: Vec<f32> = json.as_array()?.iter().map(|v| v.as_f64().map(|f| f as f32)).collect::<Option<_>>()?;
    match (alpha, list.as_slice()) {
        (false, [r, g, b]) => Some(Value::Rgb([*r, *g, *b])),
        (true, [r, g, b, a]) => Some(Value::Argb([*r, *g, *b, *a])),
        _ => None,
    }
}

fn parse_value(kind: Kind, json: &Json) -> Option<Value> {
    match kind {
        Kind::Float | Kind::Angle => json.as_f64().map(|v| Value::Float(v as f32)),
        Kind::Rgb => parse_color(json, false),
        Kind::Argb => parse_color(json, true),
        Kind::Integer => json.as_i64().map(|v| Value::Int(v as i32)),
        Kind::Boolean => json.as_bool().map(Value::Bool),
        // `TriState` values are written as flags or "default".
        Kind::Text => json.as_str().map(str::to_owned).or_else(|| json.as_bool().map(|b| b.to_string())).map(Value::Text),
        Kind::Other => Some(Value::Json(json.clone())),
    }
}

fn parse_arg(kind: Kind, op: Op, json: &Json) -> Option<Arg> {
    Some(match (op, kind) {
        (Op::Override, _) => Arg::Value(parse_value(kind, json)?),
        (Op::AlphaBlend, Kind::Float | Kind::Angle) => match json {
            Json::Number(n) => Arg::FloatAlpha(n.as_f64()? as f32, 1.0),
            _ => Arg::FloatAlpha(json.get("value")?.as_f64()? as f32, json.get("alpha").and_then(Json::as_f64).unwrap_or(1.0) as f32),
        },
        (Op::AlphaBlend, Kind::Rgb | Kind::Argb) | (Op::Multiply, Kind::Argb) => Arg::Value(parse_color(json, true)?),
        (Op::Add | Op::Subtract | Op::Multiply, Kind::Rgb) | (Op::Add | Op::Subtract, Kind::Argb) => Arg::Value(parse_color(json, false)?),
        (Op::BlendToGray, _) => Arg::Gray(json.get("brightness")?.as_f64()? as f32, json.get("factor")?.as_f64()? as f32),
        (_, Kind::Float | Kind::Angle) => Arg::Value(Value::Float(json.as_f64()? as f32)),
        (_, Kind::Integer) => Arg::Value(Value::Int(json.as_i64()? as i32)),
        (_, Kind::Boolean) => Arg::Value(Value::Bool(json.as_bool()?)),
        _ => Arg::Value(Value::Json(json.clone())),
    })
}

/// A set of attribute entries (`EnvironmentAttributeMap`), by attribute index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttributeMap {
    pub entries: HashMap<usize, Entry>,
}

/// The attribute table and name lookup shared by maps, timelines and systems.
pub struct Registry {
    pub attributes: Vec<Attribute>,
    by_name: HashMap<&'static str, usize>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        let attributes = attributes();
        let by_name = attributes.iter().enumerate().map(|(i, a)| (a.name, i)).collect();
        Self { attributes, by_name }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.by_name.get(name.trim_start_matches("minecraft:")).copied()
    }

    /// Parses an `attributes` object: bare values override, objects carry a modifier.
    pub fn parse_map(&self, json: &Json) -> Result<AttributeMap, String> {
        let mut map = AttributeMap::default();
        let Some(object) = json.as_object() else { return Ok(map) };
        for (name, value) in object {
            let Some(index) = self.index(name) else { continue };
            let kind = self.attributes[index].kind;
            let modifier = value.as_object().and_then(|o| o.get("modifier")).and_then(Json::as_str).and_then(Op::parse);
            let entry = match modifier {
                Some(op) if kind != Kind::Other || op != Op::Override => {
                    let arg = value.get("argument").and_then(|a| parse_arg(kind, op, a)).ok_or_else(|| format!("bad {name} argument"))?;
                    Entry { op, arg }
                }
                _ => Entry { op: Op::Override, arg: Arg::Value(parse_value(kind, value).ok_or_else(|| format!("bad {name} value"))?) },
            };
            map.entries.insert(index, entry);
        }
        Ok(map)
    }
}

/// `EasingType` (the named curves 26.3 data uses and cubic Béziers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Constant,
    Linear,
    Bezier { x: (f32, f32, f32), y: (f32, f32, f32) },
}

impl Ease {
    fn parse(json: Option<&Json>) -> Result<Self, String> {
        let Some(json) = json else { return Ok(Self::Linear) };
        if let Some(name) = json.as_str() {
            return match name {
                "linear" => Ok(Self::Linear),
                "constant" => Ok(Self::Constant),
                other => Err(format!("unsupported easing {other}")),
            };
        }
        let c: Vec<f32> = json.get("cubic_bezier").and_then(Json::as_array).ok_or("bad easing")?.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect();
        let [x1, y1, x2, y2] = c[..] else { return Err("cubic_bezier needs four values".into()) };
        let curve = |v1: f32, v2: f32| (3.0 * v1 - 3.0 * v2 + 1.0, -6.0 * v1 + 3.0 * v2, 3.0 * v1);
        Ok(Self::Bezier { x: curve(x1, x2), y: curve(y1, y2) })
    }

    pub fn apply(self, x: f32) -> f32 {
        match self {
            Self::Constant => 0.0,
            Self::Linear => x,
            Self::Bezier { x: xc, y: yc } => {
                let sample = |c: (f32, f32, f32), t: f32| ((c.0 * t + c.1) * t + c.2) * t;
                let gradient = |c: (f32, f32, f32), t: f32| (3.0 * c.0 * t + 2.0 * c.1) * t + c.2;
                // `CubicBezier.solveT`: Newton-Raphson, then bisection.
                let solve = || {
                    let mut t = x;
                    for _ in 0..4 {
                        let error = sample(xc, t) - x;
                        if error.abs() < 1.0e-5 {
                            return t;
                        }
                        let g = gradient(xc, t);
                        if g < 1.0e-5 {
                            break;
                        }
                        t -= (error / g).clamp(-0.25, 0.25);
                    }
                    let (mut t0, mut t1) = (0.0f32, 1.0f32);
                    while t0 < t1 {
                        let error = sample(xc, t) - x;
                        if error.abs() < 1.0e-5 {
                            return t;
                        }
                        if error < 0.0 {
                            t0 = t;
                        } else {
                            t1 = t;
                        }
                        t = (t1 + t0) / 2.0;
                    }
                    t
                };
                sample(yc, solve())
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Segment {
    ease: Ease,
    from: Arg,
    from_ticks: i32,
    to: Arg,
    to_ticks: i32,
}

/// One attribute's keyframe track in a timeline (`AttributeTrack` baked).
#[derive(Clone, Debug)]
pub struct Track {
    pub attribute: usize,
    op: Op,
    period: Option<i32>,
    segments: Vec<Segment>,
}

impl Track {
    /// `KeyframeTrackSampler.sample` then the modifier, over the clock's ticks.
    fn argument(&self, kind: Kind, ticks: i64) -> Arg {
        let t = match self.period {
            Some(p) => ticks.rem_euclid(i64::from(p)),
            None => ticks,
        };
        let segment = self.segments.iter().find(|s| t < i64::from(s.to_ticks)).unwrap_or_else(|| self.segments.last().expect("a track has segments"));
        if t <= i64::from(segment.from_ticks) {
            return segment.from.clone();
        }
        if t >= i64::from(segment.to_ticks) {
            return segment.to.clone();
        }
        let alpha = (t - i64::from(segment.from_ticks)) as f32 / (segment.to_ticks - segment.from_ticks) as f32;
        segment.from.lerp(kind, self.op, segment.ease.apply(alpha), &segment.to)
    }
}

/// A timeline: tracks sampled from one world clock (`Timeline`).
#[derive(Clone, Debug)]
pub struct Timeline {
    pub clock: String,
    pub tracks: Vec<Track>,
}

impl Timeline {
    pub fn parse(registry: &Registry, json: &Json) -> Result<Self, String> {
        let clock = json.get("clock").and_then(Json::as_str).ok_or("timeline lacks a clock")?.to_owned();
        let period = json.get("period_ticks").and_then(Json::as_i64).map(|p| p as i32);
        let mut tracks = Vec::new();
        for (name, track) in json.get("tracks").and_then(Json::as_object).into_iter().flatten() {
            let Some(attribute) = registry.index(name) else { continue };
            let kind = registry.attributes[attribute].kind;
            let op = track.get("modifier").and_then(Json::as_str).map_or(Some(Op::Override), Op::parse).ok_or("bad track modifier")?;
            let ease = Ease::parse(track.get("ease"))?;
            let keyframes: Vec<(i32, Arg)> = track
                .get("keyframes")
                .and_then(Json::as_array)
                .ok_or("track lacks keyframes")?
                .iter()
                .map(|k| Some((k.get("ticks")?.as_i64()? as i32, parse_arg(kind, op, k.get("value")?)?)))
                .collect::<Option<_>>()
                .ok_or_else(|| format!("bad keyframe in {name}"))?;
            if keyframes.is_empty() {
                return Err(format!("{name} has no keyframes"));
            }
            // `KeyframeTrackSampler.bakeSegments`.
            let segment = |from: &(i32, Arg), from_ticks: i32, to: &(i32, Arg), to_ticks: i32| Segment { ease, from: from.1.clone(), from_ticks, to: to.1.clone(), to_ticks };
            let mut segments = Vec::new();
            if keyframes.len() == 1 {
                let only = &keyframes[0];
                segments.push(Segment { ease: Ease::Constant, from: only.1.clone(), from_ticks: 0, to: only.1.clone(), to_ticks: 0 });
            } else {
                let (first, last) = (&keyframes[0], &keyframes[keyframes.len() - 1]);
                if let Some(p) = period {
                    segments.push(segment(last, last.0 - p, first, first.0));
                }
                for pair in keyframes.windows(2) {
                    segments.push(segment(&pair[0], pair[0].0, &pair[1], pair[1].0));
                }
                if let Some(p) = period {
                    segments.push(segment(last, last.0, first, first.0 + p));
                }
            }
            tracks.push(Track { attribute, op, period, segments });
        }
        Ok(Self { clock, tracks })
    }
}

/// `WeatherAttributes.RAIN` and `THUNDER`.
fn weather_maps(registry: &Registry) -> (AttributeMap, AttributeMap) {
    // `ARGB.colorFromFloat` goes through 8-bit channels.
    let c = |v: f32| (v * 255.0).floor() / 255.0;
    // `Timelines.NIGHT_SKY_LIGHT_COLOR`: colorFromFloat(1, 0.48, 0.48, 1).
    let night = [c(0.48), c(0.48), 1.0];
    let build = |sky: (f32, f32), f: f32, cloud: (f32, f32), alpha: f32| {
        let mut map = AttributeMap::default();
        let mut put = |name: &str, op: Op, arg: Arg| {
            map.entries.insert(registry.index(name).expect("weather attribute"), Entry { op, arg });
        };
        let tint = [c(f), c(f), c(f * 1.2)];
        put("visual/sky_color", Op::BlendToGray, Arg::Gray(sky.0, sky.1));
        put("visual/fog_color", Op::Multiply, Arg::Value(Value::Rgb(tint)));
        put("visual/cloud_color", Op::BlendToGray, Arg::Gray(cloud.0, cloud.1));
        put("gameplay/sky_light_level", Op::AlphaBlend, Arg::FloatAlpha(4.0, alpha));
        put("visual/sky_light_color", Op::AlphaBlend, Arg::Value(Value::Argb([night[0], night[1], night[2], c(alpha)])));
        put("visual/sky_light_factor", Op::AlphaBlend, Arg::FloatAlpha(0.24, alpha));
        put("visual/star_brightness", Op::Override, Arg::Value(Value::Float(0.0)));
        put("visual/sunrise_sunset_color", Op::Multiply, Arg::Value(Value::Argb([tint[0], tint[1], tint[2], 1.0])));
        put("gameplay/bees_stay_in_hive", Op::Override, Arg::Value(Value::Bool(true)));
        map
    };
    (build((0.6, 0.75), 0.5, (0.24, 0.5), 0.3125), build((0.24, 0.94), 0.25, (0.095, 0.94), 0.527_343_75))
}

/// The sources one dimension's attributes are layered from.
pub struct EnvironmentSystem {
    pub registry: Registry,
    /// Per attribute: the dimension type's constant applied to the default.
    base: Vec<Value>,
    /// Attribute maps of every biome, by biome ID.
    biomes: Vec<AttributeMap>,
    /// Whether any biome provides the attribute (a positional layer exists).
    biome_layer: Vec<bool>,
    timelines: Vec<Timeline>,
    weather: Option<(AttributeMap, AttributeMap)>,
}

/// Where and when to sample.
pub struct Sample<'a> {
    /// Clock totals by clock name (`ClockManager`).
    pub clock_ticks: &'a dyn Fn(&str) -> i64,
    pub rain_level: f32,
    pub thunder_level: f32,
    /// Biome weights around the camera (`SpatialAttributeInterpolator`), in
    /// sampling order; `None` samples the biome at the position.
    pub biome_weights: Option<&'a [(u16, f64)]>,
    /// The noise biome at the position.
    pub biome: u16,
}

impl EnvironmentSystem {
    /// Builds a dimension's system: `dimension_attributes` from its type,
    /// `biome_attributes` for every biome in ID order, and its timelines in
    /// tag order. `weather` is `Level.canHaveWeather`.
    pub fn new(dimension_attributes: &Json, biome_attributes: &[Json], timelines: &[Json], weather: bool) -> Result<Self, String> {
        let registry = Registry::new();
        let dimension = registry.parse_map(dimension_attributes)?;
        let base = registry
            .attributes
            .iter()
            .enumerate()
            .map(|(i, a)| dimension.entries.get(&i).map_or(a.default.clone(), |e| apply(a.kind, &a.default, e)))
            .collect();
        let biomes: Vec<AttributeMap> = biome_attributes.iter().map(|b| registry.parse_map(b)).collect::<Result<_, _>>()?;
        let mut biome_layer = vec![false; registry.attributes.len()];
        for map in &biomes {
            for &i in map.entries.keys() {
                biome_layer[i] = true;
            }
        }
        let timelines = timelines.iter().map(|t| Timeline::parse(&registry, t)).collect::<Result<_, _>>()?;
        let weather = weather.then(|| weather_maps(&registry));
        Ok(Self { registry, base, biomes, biome_layer, timelines, weather })
    }

    /// `EnvironmentAttributeSystem.getValue` for one attribute.
    pub fn value(&self, attribute: usize, sample: &Sample) -> Value {
        let info = &self.registry.attributes[attribute];
        let mut result = self.base[attribute].clone();
        if self.biome_layer[attribute] {
            result = match sample.biome_weights {
                Some(weights) if info.spatial => self.blend_biomes(attribute, &result, weights),
                _ => self.biomes.get(usize::from(sample.biome)).and_then(|m| m.entries.get(&attribute)).map_or(result.clone(), |e| apply(info.kind, &result, e)),
            };
        }
        for timeline in &self.timelines {
            for track in timeline.tracks.iter().filter(|t| t.attribute == attribute) {
                let arg = track.argument(info.kind, (sample.clock_ticks)(&timeline.clock));
                result = apply(info.kind, &result, &Entry { op: track.op, arg });
            }
        }
        if let Some((rain, thunder)) = &self.weather {
            let thunder_level = sample.thunder_level;
            let rain_level = sample.rain_level - thunder_level;
            if let Some(entry) = rain.entries.get(&attribute).filter(|_| rain_level > 0.0) {
                let v = apply(info.kind, &result, entry);
                result = lerp_value(info.kind, Purpose::StateChange, rain_level, &result, &v);
            }
            if let Some(entry) = thunder.entries.get(&attribute).filter(|_| thunder_level > 0.0) {
                let v = apply(info.kind, &result, entry);
                result = lerp_value(info.kind, Purpose::StateChange, thunder_level, &result, &v);
            }
        }
        match (&result, info.range) {
            (Value::Float(v), Some((lo, hi))) => Value::Float(v.clamp(lo, hi)),
            _ => result,
        }
    }

    pub fn value_named(&self, name: &str, sample: &Sample) -> Value {
        self.registry.index(name).map_or(Value::Json(Json::Null), |i| self.value(i, sample))
    }

    /// `SpatialAttributeInterpolator.applyAttributeLayer`.
    fn blend_biomes(&self, attribute: usize, base: &Value, weights: &[(u16, f64)]) -> Value {
        let kind = self.registry.attributes[attribute].kind;
        let source = |biome: u16| self.biomes.get(usize::from(biome)).and_then(|m| m.entries.get(&attribute)).map_or(base.clone(), |e| apply(kind, base, e));
        if weights.is_empty() {
            return base.clone();
        }
        if weights.len() == 1 {
            return source(weights[0].0);
        }
        let (mut result, mut accumulated): (Option<Value>, f64) = (None, 0.0);
        for &(biome, weight) in weights {
            let value = source(biome);
            accumulated += weight;
            result = Some(match result {
                None => value,
                Some(r) => lerp_value(kind, Purpose::Spatial, (weight / accumulated) as f32, &r, &value),
            });
        }
        result.expect("weights are non-empty")
    }
}

/// `GaussianSampler.sample` of noise biomes around a quart position
/// (`EnvironmentAttributeProbe.tick`: `position.scale(0.25)`), merged by
/// biome in first-sample order.
pub fn biome_weights(quart_pos: [f64; 3], biome_at: impl Fn(i32, i32, i32) -> u16) -> Vec<(u16, f64)> {
    const KERNEL: [f64; 7] = [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0];
    let p = [quart_pos[0] - 0.5, quart_pos[1] - 0.5, quart_pos[2] - 0.5];
    let i = p.map(|v| v.floor() as i32);
    let r = [p[0] - f64::from(i[0]), p[1] - f64::from(i[1]), p[2] - f64::from(i[2])];
    let w = |k: usize, rel: f64| KERNEL[k + 1] + rel * (KERNEL[k] - KERNEL[k + 1]);
    let mut out: Vec<(u16, f64)> = Vec::new();
    for z in 0..6 {
        let wz = w(z, r[2]);
        for x in 0..6 {
            let wx = w(x, r[0]);
            for y in 0..6 {
                let wy = w(y, r[1]);
                let biome = biome_at(i[0] - 2 + x as i32, i[1] - 2 + y as i32, i[2] - 2 + z as i32);
                let weight = wx * wy * wz;
                match out.iter_mut().find(|(b, _)| *b == biome) {
                    Some(entry) => entry.1 += weight,
                    None => out.push((biome, weight)),
                }
            }
        }
    }
    out
}

/// `DimensionType.Skybox`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skybox {
    Overworld,
    End,
    None,
}

/// A dimension type's presentation settings beside its attributes.
#[derive(Clone, Debug)]
pub struct DimensionPresentation {
    pub skybox: Skybox,
    /// `DimensionType.defaultClock`, the clock `/time` reports.
    pub default_clock: Option<String>,
    pub min_y: i32,
    /// `DimensionType.cardinalLightType` is `nether`: entities take the
    /// second diffuse light from below.
    pub nether_cardinal_light: bool,
}

impl EnvironmentSystem {
    /// Loads a dimension type's system from the data pack: its attributes,
    /// its timelines tag and every biome's attributes in ID order. Weather
    /// applies where `Level.canHaveWeather` holds (skylight, no ceiling,
    /// not the End).
    pub fn for_dimension(registries: &crate::Registries, dimension_type: &str) -> Result<(Self, DimensionPresentation), String> {
        let pack = &registries.datapack;
        let id = crate::ident::Identifier::parse(dimension_type)?;
        let json = pack.read_json("dimension_type", &id)?;
        let mut timelines = Vec::new();
        match json.get("timelines") {
            Some(Json::String(text)) => resolve_timelines(pack, text, &mut timelines, 0)?,
            Some(Json::Array(list)) => {
                for entry in list.iter().filter_map(Json::as_str) {
                    resolve_timelines(pack, entry, &mut timelines, 0)?;
                }
            }
            _ => {}
        }
        let biomes: Vec<Json> = registries.biomes.iter().map(|(_, info)| info.attributes.clone()).collect();
        let flag = |key: &str| json.get(key).and_then(Json::as_bool).unwrap_or(false);
        let weather = flag("has_skylight") && !flag("has_ceiling") && id.as_str() != "minecraft:the_end";
        let timelines: Vec<Json> = timelines.into_iter().map(|(_, json)| json).collect();
        let system = Self::new(json.get("attributes").unwrap_or(&Json::Null), &biomes, &timelines, weather)?;
        let skybox = match json.get("skybox").and_then(Json::as_str) {
            Some("end") => Skybox::End,
            Some("none") => Skybox::None,
            _ => Skybox::Overworld,
        };
        let presentation = DimensionPresentation {
            skybox,
            default_clock: json.get("default_clock").and_then(Json::as_str).map(str::to_owned),
            min_y: json.get("min_y").and_then(Json::as_i64).unwrap_or(0) as i32,
            nether_cardinal_light: json.get("cardinal_light").and_then(Json::as_str) == Some("nether"),
        };
        Ok((system, presentation))
    }
}

/// Appends a timeline or a `#tag`'s timelines in tag order, skipping
/// duplicates as a `HolderSet` does.
fn resolve_timelines(pack: &crate::datapack::DataPack, entry: &str, out: &mut Vec<(crate::ident::Identifier, Json)>, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err(format!("timeline tag {entry} nests too deeply"));
    }
    if let Some(tag) = entry.strip_prefix('#') {
        let tag = pack.read_json("tags/timeline", &crate::ident::Identifier::parse(tag)?)?;
        for value in tag.get("values").and_then(Json::as_array).into_iter().flatten() {
            let name = value.as_str().or_else(|| value.get("id").and_then(Json::as_str)).ok_or("bad timeline tag entry")?;
            resolve_timelines(pack, name, out, depth + 1)?;
        }
        return Ok(());
    }
    let id = crate::ident::Identifier::parse(entry)?;
    if !out.iter().any(|(seen, _)| *seen == id) {
        let json = pack.read_json("timeline", &id)?;
        out.push((id, json));
    }
    Ok(())
}
