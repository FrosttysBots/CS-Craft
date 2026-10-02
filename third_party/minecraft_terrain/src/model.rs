use crate::{
    pack::{PackStack, ResourceId},
    scene::Block,
};
use anyhow::{anyhow, bail, Result};
use glam::{EulerRot, Mat4, Quat, Vec3};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug)]
pub struct Face {
    pub direction: String,
    pub texture: ResourceId,
    pub uv: [f32; 4],
    pub cull: bool,
    /// The named `cullface`, when it differs from the face's own direction.
    pub cullface: Option<String>,
    pub tint: bool,
    pub tint_index: Option<usize>,
    pub force_translucent: bool,
}
#[derive(Clone, Debug)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub faces: Vec<Face>,
    pub rotation_y: u16,
    /// `CuboidModelElement.rotation`, applied before the blockstate's.
    pub rotation: Option<ElementRotation>,
    /// CuboidModelElement.shadeDirectionOverride affects directional brightness.
    pub shade_direction_override: Option<String>,
}

/// `CuboidRotation`: an element's rotation about its origin (in block
/// units), with `rescale` folded into the matrix as vanilla does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementRotation {
    pub origin: [f32; 3],
    /// JOML's column-major `m[column][row]`.
    pub matrix: [[f32; 3]; 3],
    /// Blockstate X rotation, applied after the element's local rotation.
    pub block_x: u16,
}

impl ElementRotation {
    /// `FaceBakery.rotateVertexBy`: about the origin, with JOML's unfused
    /// `transformPosition`.
    pub fn apply(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
        let (x, y, z) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
        let m = &self.matrix;
        let row = |r: usize| m[0][r] * x + (m[1][r] * y + (m[2][r] * z + 0.0));
        rotate_block_x([row(0) + self.origin[0], row(1) + self.origin[1], row(2) + self.origin[2]],self.block_x)
    }

    /// `CuboidModelElement.Deserializer.getRotation`: `axis` and `angle`, or
    /// Euler `x`, `y` and `z`, about `origin` (in sixteenths), optionally
    /// rescaled so the rotated element keeps its extent.
    pub fn parse(raw: &Value) -> Result<Option<Self>> {
        let Some(rotation) = raw.get("rotation") else { return Ok(None) };
        let origin = rotation.get("origin").map(coords).transpose()?.unwrap_or([0.0; 3]);
        let number = |key: &str| rotation.get(key).and_then(Value::as_f64).map(|v| v as f32);
        let radians = |degrees: f32| degrees * (std::f64::consts::PI / 180.0) as f32;
        let mut m = if rotation.get("axis").is_some() || rotation.get("angle").is_some() {
            let angle = number("angle").ok_or_else(|| anyhow!("rotation angle missing"))?;
            let axis = rotation.get("axis").and_then(Value::as_str).ok_or_else(|| anyhow!("rotation axis missing"))?;
            single_axis(axis, radians(angle))?
        } else if ["x", "y", "z"].iter().any(|k| rotation.get(*k).is_some()) {
            euler_zyx(radians(number("z").unwrap_or(0.0)), radians(number("y").unwrap_or(0.0)), radians(number("x").unwrap_or(0.0)))
        } else {
            bail!("rotation needs an axis and angle or x, y and z");
        };
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        if rotation.get("rescale").and_then(Value::as_bool).unwrap_or(false) && m != identity {
            // `computeRescale`: each axis by one over its image's largest
            // component, as `Matrix4f.scale` scales the columns.
            for column in &mut m {
                let largest = column.iter().fold(0.0f32, |a, v| a.max(v.abs()));
                let factor = 1.0 / largest;
                for v in column.iter_mut() {
                    *v *= factor;
                }
            }
        }
        Ok(Some(Self { origin, matrix: m, block_x:0 }))
    }
}

/// JOML `Math.cosFromSin`.
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0 - sin * sin).sqrt();
    let pi2 = (std::f64::consts::PI * 2.0) as f32;
    let a = angle + std::f32::consts::FRAC_PI_2;
    let mut b = a - (a / pi2) as i32 as f32 * pi2;
    if b < 0.0 {
        b += pi2;
    }
    if b >= std::f32::consts::PI { -cos } else { cos }
}

/// `Matrix4f.rotation(angle, axis)` for a unit axis: `rotationX`, `rotationY`
/// or `rotationZ`.
fn single_axis(axis: &str, angle: f32) -> Result<[[f32; 3]; 3]> {
    let sin = (angle as f64).sin() as f32;
    let cos = cos_from_sin(sin, angle);
    Ok(match axis {
        "x" => [[1.0, 0.0, 0.0], [0.0, cos, sin], [0.0, -sin, cos]],
        "y" => [[cos, 0.0, -sin], [0.0, 1.0, 0.0], [sin, 0.0, cos]],
        "z" => [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]],
        _ => bail!("invalid rotation axis {axis}"),
    })
}

/// `Matrix4f.rotationZYX(angleZ, angleY, angleX)`.
fn euler_zyx(z: f32, y: f32, x: f32) -> [[f32; 3]; 3] {
    let (sin_x, sin_y, sin_z) = ((x as f64).sin() as f32, (y as f64).sin() as f32, (z as f64).sin() as f32);
    let (cos_x, cos_y, cos_z) = (cos_from_sin(sin_x, x), cos_from_sin(sin_y, y), cos_from_sin(sin_z, z));
    let (nm00, nm01, nm10, nm11) = (cos_z, sin_z, -sin_z, cos_z);
    let (nm20, nm21, nm22) = (nm00 * sin_y, nm01 * sin_y, cos_y);
    [
        [nm00 * cos_y, nm01 * cos_y, -sin_y],
        [nm10 * cos_x + nm20 * sin_x, nm11 * cos_x + nm21 * sin_x, nm22 * sin_x],
        [nm10 * -sin_x + nm20 * cos_x, nm11 * -sin_x + nm21 * cos_x, nm22 * cos_x],
    ]
}
#[derive(Clone, Debug)]
pub struct ResolvedModel {
    pub elements: Vec<Element>,
}
/// The held item uses its 26.3 item definition's model, not a placed
/// blockstate (whose defaults and shape may differ from the inventory model).
pub fn resolve_item_model(pack: &PackStack, item: &ResourceId) -> Result<Option<ResolvedModel>> {
    if item.path == "chest" {
        return closed_chest_model(&Block::new(&item.key())).map(Some);
    }
    // Beds are a two-part composite in 26.3, with the foot translated one block.
    if item.path.ends_with("_bed") {
        if let Some(definition) = pack.item_definition(item)? {
            if let Some(parts) = definition["model"]["models"].as_array() {
                let mut elements = Vec::new();
                for part in parts {
                    let Some(reference) = part["model"].as_str() else {continue};
                    let choice = serde_json::json!({"model":reference});
                    let mut model = resolve_choices(pack, &Block::new(&item.key()), &[&choice])?;
                    let offset: [f32;3] = std::array::from_fn(|i|part["transformation"]["translation"][i].as_f64().unwrap_or(0.) as f32);
                    for element in &mut model.elements {
                        for i in 0..3 {element.from[i]+=offset[i];element.to[i]+=offset[i];}
                        if let Some(rotation)=&mut element.rotation {for i in 0..3 {rotation.origin[i]+=offset[i];}}
                    }
                    elements.extend(model.elements);
                }
                return Ok(Some(ResolvedModel {elements}));
            }
        }
    }
    let Some(definition) = pack.item_definition(item)? else {
        return Ok(None);
    };
    let Some(reference) = crate::interface::item_model_reference(&definition["model"]) else {
        return Ok(None);
    };
    let model_id = ResourceId::parse(reference)?;
    let inherited = load_parented(pack, &model_id, &mut HashSet::new())?;
    if inherited.get("builtin").and_then(Value::as_str) == Some("generated") {
        return generated_item_model(pack, &model_id, &inherited).map(Some);
    }
    let choice = serde_json::json!({"model": reference});
    let block = Block::new(&item.key());
    resolve_choices(pack, &block, &[&choice]).map(Some)
}
/// ItemTransform.apply for the right hand, using the selected item's inherited
/// display transform from the resource pack.
pub fn item_first_person_transform(pack: &PackStack, item: &ResourceId) -> Result<Mat4> {
    item_display_transform(pack, item, "firstperson_righthand")
}
pub fn item_display_transform(pack: &PackStack, item: &ResourceId, context: &str) -> Result<Mat4> {
    let Some(definition) = pack.item_definition(item)? else {
        return Ok(Mat4::IDENTITY);
    };
    let Some(reference) = crate::interface::item_model_reference(&definition["model"]).or_else(||(item.path=="chest").then_some("minecraft:item/chest")) else {
        return Ok(Mat4::IDENTITY);
    };
    let model = load_parented(pack, &ResourceId::parse(reference)?, &mut HashSet::new())?;
    let display = &model["display"][context];
    let vector = |key: &str, default: Vec3| -> Vec3 {
        display[key]
            .as_array()
            .filter(|array| array.len() == 3)
            .map(|array| {
                Vec3::new(
                    array[0].as_f64().unwrap_or(default.x as f64) as f32,
                    array[1].as_f64().unwrap_or(default.y as f64) as f32,
                    array[2].as_f64().unwrap_or(default.z as f64) as f32,
                )
            })
            .unwrap_or(default)
    };
    let degrees = vector("rotation", Vec3::ZERO);
    let rotation = Vec3::new(
        degrees.x.to_radians(),
        degrees.y.to_radians(),
        degrees.z.to_radians(),
    );
    let translation =
        vector("translation", Vec3::ZERO).clamp(Vec3::splat(-80.0), Vec3::splat(80.0)) / 16.0;
    let scale = vector("scale", Vec3::ONE).clamp(Vec3::splat(-4.0), Vec3::splat(4.0));
    Ok(Mat4::from_translation(translation)
        * Mat4::from_quat(Quat::from_euler(
            EulerRot::XYZ,
            rotation.x,
            rotation.y,
            rotation.z,
        ))
        * Mat4::from_scale(scale)
        * Mat4::from_translation(Vec3::splat(-0.5)))
}
fn generated_item_model(pack: &PackStack, id: &ResourceId, model: &Value) -> Result<ResolvedModel> {
    let textures = model
        .get("textures")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("generated item has no textures"))?;
    let mut elements = Vec::new();
    for layer in 0..5 {
        let key = format!("layer{layer}");
        if !textures.contains_key(&key) {
            continue;
        }
        let texture = resolve_texture(&format!("#{key}"), textures, &id.namespace)?;
        let bytes = pack
            .texture(&texture)?
            .ok_or_else(|| anyhow!("missing generated item sprite {}", texture.key()))?;
        let sprite = image::load_from_memory(&bytes)?.to_rgba8();
        let w = sprite.width().min(sprite.height());
        let h = w;
        if w == 0 {
            continue;
        }
        let face = |direction: &str, uv: [f32; 4]| Face {
            direction: direction.into(),
            texture: texture.clone(),
            uv,
            cull: false,
            cullface: None,
            tint: true,
            tint_index: Some(layer),
            force_translucent: false,
        };
        elements.push(Element {
            from: [0.0, 0.0, 7.5 / 16.0],
            to: [1.0, 1.0, 8.5 / 16.0],
            rotation_y: 0,
            rotation: None,
            shade_direction_override: None,
            faces: vec![
                face("south", [0.0, 0.0, 1.0, 1.0]),
                face("north", [1.0, 0.0, 0.0, 1.0]),
            ],
        });
        for y in 0..h {
            for x in 0..w {
                if sprite.get_pixel(x, y)[3] == 0 {
                    continue;
                }
                let opaque = |xx: i32, yy: i32| -> bool {
                    xx >= 0
                        && yy >= 0
                        && xx < w as i32
                        && yy < h as i32
                        && sprite.get_pixel(xx as u32, yy as u32)[3] != 0
                };
                let left = x as f32 / w as f32;
                let right = (x + 1) as f32 / w as f32;
                let top = 1.0 - y as f32 / h as f32;
                let bottom = 1.0 - (y + 1) as f32 / h as f32;
                let u0 = (x as f32 + 0.1) / w as f32;
                let u1 = (x as f32 + 0.9) / w as f32;
                let v0 = (y as f32 + 0.1) / h as f32;
                let v1 = (y as f32 + 0.9) / h as f32;
                let mut side = |from, to, direction, uv| {
                    elements.push(Element {
                        from,
                        to,
                        rotation_y: 0,
                        rotation: None,
                        shade_direction_override: None,
                        faces: vec![face(direction, uv)],
                    })
                };
                if !opaque(x as i32, y as i32 - 1) {
                    side(
                        [left, top, 7.5 / 16.0],
                        [right, top, 8.5 / 16.0],
                        "up",
                        [u0, v1, u1, v0],
                    );
                }
                if !opaque(x as i32, y as i32 + 1) {
                    side(
                        [left, bottom, 7.5 / 16.0],
                        [right, bottom, 8.5 / 16.0],
                        "down",
                        [u0, v1, u1, v0],
                    );
                }
                if !opaque(x as i32 - 1, y as i32) {
                    side(
                        [left, top, 7.5 / 16.0],
                        [left, bottom, 8.5 / 16.0],
                        "east",
                        [u0, v0, u1, v1],
                    );
                }
                if !opaque(x as i32 + 1, y as i32) {
                    side(
                        [right, top, 7.5 / 16.0],
                        [right, bottom, 8.5 / 16.0],
                        "west",
                        [u0, v0, u1, v1],
                    );
                }
            }
        }
    }
    Ok(ResolvedModel { elements })
}

pub fn resolve_block(pack: &PackStack, block: &Block) -> Result<ResolvedModel> {
    if block.id.path=="end_portal" {return end_portal_model();}
    if block.id.path == "chest" {
        return closed_chest_model(block);
    }
    let state = pack
        .blockstate(&block.id)?
        .ok_or_else(|| anyhow!("missing blockstate {}", block.id.key()))?;
    let choices = choose_models(&state, &block.properties)?;
    resolve_choices(pack, block, &choices)
}
/// Particle material selected by the block model, including inherited
/// `textures.particle` references (grass blocks deliberately use dirt here).
pub fn block_particle_texture(pack: &PackStack, block: &Block) -> Result<Option<ResourceId>> {
    let Some(state) = pack.blockstate(&block.id)? else {
        return Ok(None);
    };
    let Some(choice) = choose_models(&state, &block.properties)?.first().copied() else {
        return Ok(None);
    };
    let Some(model_name) = choice.get("model").and_then(Value::as_str) else {
        return Ok(None);
    };
    let model_id = ResourceId::parse(model_name)?;
    let model = load_parented(pack, &model_id, &mut HashSet::new())?;
    let Some(textures) = model.get("textures").and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(particle) = textures.get("particle").and_then(Value::as_str) else {
        return Ok(None);
    };
    resolve_texture(particle, textures, &model_id.namespace).map(Some)
}

/// Resolve each weighted blockstate alternative once. The mesh picks one
/// using Minecraft's position seed; the pack may replace these choices.
pub fn resolve_block_variants(
    pack: &PackStack,
    block: &Block,
) -> Result<Vec<(ResolvedModel, u32)>> {
    if block.id.path=="end_portal" {return Ok(vec![(end_portal_model()?,1)]);}
    if block.id.path == "chest" {
        return Ok(vec![(closed_chest_model(block)?, 1)]);
    }
    let state = pack
        .blockstate(&block.id)?
        .ok_or_else(|| anyhow!("missing blockstate {}", block.id.key()))?;
    if let Some(value) = selected_variant(&state, &block.properties)? {
        if let Some(choices) = value.as_array() {
            if choices.is_empty() {
                bail!("empty weighted model list");
            }
            let mut total = 0u32;
            return choices
                .iter()
                .map(|choice| {
                    let weight = choice.get("weight").and_then(Value::as_u64).unwrap_or(1);
                    let weight =
                        u32::try_from(weight).map_err(|_| anyhow!("model weight too large"))?;
                    if weight == 0 {
                        bail!("model weight must be positive");
                    }
                    total = total
                        .checked_add(weight)
                        .filter(|&sum| sum <= i32::MAX as u32)
                        .ok_or_else(|| anyhow!("total model weight too large"))?;
                    Ok((resolve_choices(pack, block, &[choice])?, weight))
                })
                .collect();
        }
    }
    Ok(vec![(
        resolve_choices(pack, block, &choose_models(&state, &block.properties)?)?,
        1,
    )])
}

/// Closed single chest from 26.3 ChestModel.createSingleBodyLayer. Vanilla
/// uses a block-entity renderer for this model; this static mesh covers its
/// closed pose until opening animation is represented in the world pass.
fn closed_chest_model(block: &Block) -> Result<ResolvedModel> {
    let chest_type = block
        .properties
        .get("type")
        .map(String::as_str)
        .unwrap_or("single");
    let texture = ResourceId::parse(match chest_type {
        "left" => "minecraft:entity/chest/normal_left",
        "right" => "minecraft:entity/chest/normal_right",
        _ => "minecraft:entity/chest/normal",
    })?;
    let omitted_side = match chest_type {
        "left" => Some("west"),
        "right" => Some("east"),
        _ => None,
    };
    let (body_x0, body_x1, lock_x0, lock_x1) = match chest_type {
        "left" => (0.0, 15.0, 0.0, 1.0),
        "right" => (1.0, 16.0, 15.0, 16.0),
        _ => (1.0, 15.0, 7.0, 9.0),
    };
    let rotation_y = match block
        .properties
        .get("facing")
        .map(String::as_str)
        .unwrap_or("north")
    {
        "north" => 180,
        "east" => 270,
        "south" => 0,
        "west" => 90,
        _ => 180,
    };
    let make_box = |from: [f32; 3], to: [f32; 3], u: f32, v: f32| {
        let width = (to[0] - from[0]) * 16.0;
        let height = (to[1] - from[1]) * 16.0;
        let depth = (to[2] - from[2]) * 16.0;
        let rect = |a: f32, b: f32, c: f32, d: f32| [a / 64.0, b / 64.0, c / 64.0, d / 64.0];
        Element {
            from,
            to,
            rotation_y,
            rotation: None,
            shade_direction_override: None,
            faces: [
                ("down", rect(u + depth, v, u + depth + width, v + depth)),
                (
                    "up",
                    rect(u + depth + width, v, u + depth + width * 2.0, v + depth),
                ),
                (
                    "north",
                    rect(u + depth, v + depth, u + depth + width, v + depth + height),
                ),
                (
                    "south",
                    rect(
                        u + depth * 2.0 + width,
                        v + depth,
                        u + depth * 2.0 + width * 2.0,
                        v + depth + height,
                    ),
                ),
                ("west", rect(u, v + depth, u + depth, v + depth + height)),
                (
                    "east",
                    rect(
                        u + depth + width,
                        v + depth,
                        u + depth * 2.0 + width,
                        v + depth + height,
                    ),
                ),
            ]
            .into_iter()
            .filter(|(direction, _)| Some(*direction) != omitted_side)
            .map(|(direction, uv)| {
                // ModelPart.Cube's positive Y is world-up for this block
                // entity. Its side polygons assign both U and V opposite to
                // the block-mesh corner order; the top reverses V only.
                let uv = match direction {
                    "north" | "south" | "west" | "east" => [uv[2], uv[3], uv[0], uv[1]],
                    "up" => [uv[0], uv[3], uv[2], uv[1]],
                    _ => uv,
                };
                // The entity chest sheets have transparent texels immediately
                // outside several faces. Keep the shared world-atlas sampler
                // inside the face at magnification, including its join edge.
                let inset = 0.25 / 64.0;
                let inset_axis = |a: f32, b: f32| {
                    if a < b {
                        (a + inset, b - inset)
                    } else {
                        (a - inset, b + inset)
                    }
                };
                let (u0, u1) = inset_axis(uv[0], uv[2]);
                Face {
                    direction: direction.into(),
                    texture: texture.clone(),
                    uv: [u0, uv[1], u1, uv[3]],
                    cull: false,
                    cullface: None,
                    tint: false,
                    tint_index: None,
                    force_translucent: false,
                }
            })
            .collect(),
        }
    };
    Ok(ResolvedModel {
        elements: vec![
            make_box(
                [body_x0 / 16.0, 0.0, 1.0 / 16.0],
                [body_x1 / 16.0, 10.0 / 16.0, 15.0 / 16.0],
                0.0,
                19.0,
            ),
            make_box(
                [body_x0 / 16.0, 9.0 / 16.0, 1.0 / 16.0],
                [body_x1 / 16.0, 14.0 / 16.0, 15.0 / 16.0],
                0.0,
                0.0,
            ),
            make_box(
                [lock_x0 / 16.0, 7.0 / 16.0, 15.0 / 16.0],
                [lock_x1 / 16.0, 11.0 / 16.0, 1.0],
                0.0,
                0.0,
            ),
        ],
    })
}

fn resolve_choices(pack: &PackStack, block: &Block, choices: &[&Value]) -> Result<ResolvedModel> {
    let mut elements = Vec::new();
    for &choice in choices {
        let rotation_x=choice.get("x").and_then(Value::as_u64).unwrap_or(0);
        if rotation_x>270 || rotation_x%90!=0 {bail!("unsupported model X rotation {rotation_x}");}
        let rotation_y = choice.get("y").and_then(Value::as_u64).unwrap_or(0);
        if rotation_y > 270 || rotation_y % 90 != 0 {
            bail!("unsupported model Y rotation {rotation_y}");
        }
        let raw_id = choice
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("blockstate model missing"))?;
        let model_id = ResourceId::parse(raw_id)?;
        let mut seen = HashSet::new();
        let model = load_parented(pack, &model_id, &mut seen)?;
        let textures = model
            .get("textures")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(raw_elements) = model.get("elements").and_then(Value::as_array) else {
            // Fluid block models deliberately contain only a particle texture in vanilla.
            if matches!(block.id.path.as_str(), "water" | "lava") {
                elements.extend(fallback_fluid(ResourceId::parse(&format!(
                    "minecraft:block/{}_still",
                    block.id.path
                ))?));
            }
            continue;
        };
        for raw in raw_elements {
            let from = coords(
                raw.get("from")
                    .ok_or_else(|| anyhow!("model element from missing"))?,
            )?;
            let to = coords(
                raw.get("to")
                    .ok_or_else(|| anyhow!("model element to missing"))?,
            )?;
            let faces = raw
                .get("faces")
                .and_then(Value::as_object)
                .ok_or_else(|| anyhow!("model faces missing"))?;
            let mut resolved = Vec::new();
            for (direction, data) in faces {
                let texture = data
                    .get("texture")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("model face texture missing"))?;
                let (texture, force_translucent) =
                    resolve_texture_material(texture, &textures, &model_id.namespace)?;
                let uv = data
                    .get("uv")
                    .map(uv_coords)
                    .transpose()?
                    .unwrap_or(implicit_uv(direction, from, to)?);
                resolved.push(Face {
                    direction: direction.clone(),
                    texture,
                    uv,
                    cull: data.get("cullface").is_some(),
                    cullface: data.get("cullface").and_then(Value::as_str).map(|dir|rotate_x_direction(dir,rotation_x as u16).to_string()),
                    tint: data.get("tintindex").is_some(),
                    tint_index: data
                        .get("tintindex")
                        .and_then(Value::as_u64)
                        .map(|index| index as usize),
                    force_translucent,
                });
            }
            elements.push(Element {
                from,
                to,
                faces: resolved,
                rotation_y: rotation_y as u16,
                rotation: {
                    let mut r=ElementRotation::parse(raw)?;
                    if rotation_x!=0 {
                        let value=r.get_or_insert(ElementRotation{origin:[0.5;3],matrix:[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],block_x:0});
                        value.block_x=rotation_x as u16;
                    }
                    r
                },
                shade_direction_override: raw
                    .get("shade_direction_override")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
    }
    Ok(ResolvedModel { elements })
}
pub fn rotate_block_x([x,y,z]:[f32;3],degrees:u16)->[f32;3]{
    match degrees {90=>[x,z,1.-y],180=>[x,1.-y,1.-z],270=>[x,1.-z,y],_=>[x,y,z]}
}
fn rotate_x_direction(dir:&str,degrees:u16)->&str {
    let cycle=["north","down","south","up"];
    cycle.iter().position(|v|*v==dir).map_or(dir,|i|cycle[(i+degrees as usize/90)%4])
}
fn coords(v: &Value) -> Result<[f32; 3]> {
    let a = v
        .as_array()
        .ok_or_else(|| anyhow!("model coordinates must be array"))?;
    if a.len() != 3 {
        bail!("model coordinates must have 3 elements");
    }
    Ok([
        a[0].as_f64().ok_or_else(|| anyhow!("bad coordinate"))? as f32 / 16.0,
        a[1].as_f64().ok_or_else(|| anyhow!("bad coordinate"))? as f32 / 16.0,
        a[2].as_f64().ok_or_else(|| anyhow!("bad coordinate"))? as f32 / 16.0,
    ])
}
fn uv_coords(v: &Value) -> Result<[f32; 4]> {
    let a = v.as_array().ok_or_else(|| anyhow!("UV must be array"))?;
    if a.len() != 4 {
        bail!("UV needs 4 elements");
    }
    Ok([
        a[0].as_f64().ok_or_else(|| anyhow!("bad UV"))? as f32 / 16.0,
        a[1].as_f64().ok_or_else(|| anyhow!("bad UV"))? as f32 / 16.0,
        a[2].as_f64().ok_or_else(|| anyhow!("bad UV"))? as f32 / 16.0,
        a[3].as_f64().ok_or_else(|| anyhow!("bad UV"))? as f32 / 16.0,
    ])
}
/// Minecraft's omitted face UVs are derived from the element bounds in 0..16
/// model units. `from` and `to` are already divided by 16, so these values
/// share the normalized tile coordinate space used by explicit UVs.
fn implicit_uv(direction: &str, from: [f32; 3], to: [f32; 3]) -> Result<[f32; 4]> {
    let [x, y, z] = from;
    let [xx, yy, zz] = to;
    Ok(match direction {
        "down" => [x, 1.0 - zz, xx, 1.0 - z],
        "up" => [x, z, xx, zz],
        "north" => [1.0 - xx, 1.0 - yy, 1.0 - x, 1.0 - y],
        "south" => [x, 1.0 - yy, xx, 1.0 - y],
        "west" => [z, 1.0 - yy, zz, 1.0 - y],
        "east" => [1.0 - zz, 1.0 - yy, 1.0 - z, 1.0 - y],
        _ => bail!("unsupported model face direction {direction}"),
    })
}
fn resolve_texture(
    raw: &str,
    textures: &serde_json::Map<String, Value>,
    namespace: &str,
) -> Result<ResourceId> {
    resolve_texture_material(raw, textures, namespace).map(|(texture, _)| texture)
}
fn resolve_texture_material(
    raw: &str,
    textures: &serde_json::Map<String, Value>,
    namespace: &str,
) -> Result<(ResourceId, bool)> {
    let mut key = raw.to_owned();
    let mut force_translucent = false;
    for _ in 0..16 {
        if !key.starts_with('#') {
            let qualified = if key.contains(':') {
                key.clone()
            } else {
                format!("{namespace}:{key}")
            };
            return Ok((ResourceId::parse(&qualified)?, force_translucent));
        }
        let value = textures
            .get(&key[1..])
            .ok_or_else(|| anyhow!("missing texture variable {key}"))?;
        force_translucent |= value
            .get("force_translucent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        key = value
            .as_str()
            .or_else(|| value.get("sprite").and_then(Value::as_str))
            .ok_or_else(|| anyhow!("invalid texture value {key}"))?
            .to_owned();
    }
    bail!("texture variable cycle")
}
fn load_parented(
    pack: &PackStack,
    id: &ResourceId,
    seen: &mut HashSet<ResourceId>,
) -> Result<Value> {
    if !seen.insert(id.clone()) {
        bail!("model inheritance cycle at {}", id.key());
    }
    if id.path == "builtin/generated" {
        seen.remove(id);
        return Ok(serde_json::json!({"builtin":"generated"}));
    }
    let mut value = pack
        .model(id)?
        .ok_or_else(|| anyhow!("missing model {}", id.key()))?;
    if let Some(parent) = value.get("parent").and_then(Value::as_str) {
        let parent_id = ResourceId::parse(parent)?;
        let mut inherited = load_parented(pack, &parent_id, seen)?;
        let own = value
            .as_object_mut()
            .ok_or_else(|| anyhow!("model must be object"))?;
        let base = inherited
            .as_object_mut()
            .ok_or_else(|| anyhow!("parent model must be object"))?;
        if let Some(own_textures) = own.remove("textures") {
            let dest = base
                .entry("textures")
                .or_insert_with(|| Value::Object(Default::default()));
            for (key, val) in own_textures
                .as_object()
                .ok_or_else(|| anyhow!("textures must be object"))?
            {
                dest.as_object_mut()
                    .ok_or_else(|| anyhow!("textures must be object"))?
                    .insert(key.clone(), val.clone());
            }
        }
        if let Some(own_display) = own.remove("display") {
            let dest = base
                .entry("display")
                .or_insert_with(|| Value::Object(Default::default()));
            for (key, val) in own_display
                .as_object()
                .ok_or_else(|| anyhow!("display must be object"))?
            {
                dest.as_object_mut()
                    .ok_or_else(|| anyhow!("display must be object"))?
                    .insert(key.clone(), val.clone());
            }
        }
        for (key, val) in own {
            base.insert(key.clone(), val.clone());
        }
        value = inherited;
    }
    seen.remove(id);
    Ok(value)
}
fn choose_models<'a>(
    state: &'a Value,
    properties: &BTreeMap<String, String>,
) -> Result<Vec<&'a Value>> {
    if let Some(selected) = selected_variant(state, properties)? {
        return Ok(vec![choice(selected)?]);
    }
    if let Some(parts) = state.get("multipart").and_then(Value::as_array) {
        let mut choices = Vec::new();
        for part in parts {
            if part
                .get("when")
                .map_or(true, |w| when_matches(w, properties))
            {
                choices.push(choice(
                    part.get("apply")
                        .ok_or_else(|| anyhow!("multipart apply missing"))?,
                )?);
            }
        }
        return Ok(choices);
    }
    bail!("blockstate needs variants or multipart")
}
fn selected_variant<'a>(
    state: &'a Value,
    properties: &BTreeMap<String, String>,
) -> Result<Option<&'a Value>> {
    let Some(variants) = state.get("variants").and_then(Value::as_object) else {
        return Ok(None);
    };
    variants
        .iter()
        .find(|(key, _)| {
            key.is_empty()
                || key.split(',').all(|pair| {
                    pair.split_once('=').is_some_and(|(p, v)| {
                        properties
                            .get(p)
                            .is_some_and(|actual| v.split('|').any(|option| option == actual))
                    })
                })
        })
        .map(|(_, value)| Some(value))
        .ok_or_else(|| anyhow!("no matching blockstate variant"))
}
fn choice(v: &Value) -> Result<&Value> {
    if let Some(a) = v.as_array() {
        a.first()
            .ok_or_else(|| anyhow!("empty weighted model list"))
    } else {
        Ok(v)
    }
}
fn when_matches(v: &Value, props: &BTreeMap<String, String>) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if let Some(or) = obj.get("OR").and_then(Value::as_array) {
        return or.iter().any(|v| when_matches(v, props));
    }
    if let Some(and) = obj.get("AND").and_then(Value::as_array) {
        return and.iter().all(|v| when_matches(v, props));
    }
    obj.iter().all(|(k, v)| {
        v.as_str().is_some_and(|wanted| {
            props
                .get(k)
                .is_some_and(|actual| wanted.split('|').any(|w| w == actual))
        })
    })
}
fn fallback_fluid(texture: ResourceId) -> Vec<Element> {
    vec![Element {
        from: [0.0; 3],
        to: [1.0, 8.0 / 9.0, 1.0],
        rotation_y: 0,
        rotation: None,
        shade_direction_override: None,
        faces: ["down", "up", "north", "south", "west", "east"]
            .into_iter()
            .map(|d| Face {
                direction: d.into(),
                texture: texture.clone(),
                uv: [0.0, 0.0, 1.0, 1.0],
                cull: true,
                cullface: None,
                tint: false,
                tint_index: None,
                force_translucent: false,
            })
            .collect(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::fs;
    #[test]
    fn texture_material_preserves_forced_translucency_through_reference() {
        let textures = serde_json::json!({
            "line": "#material",
            "material": {"sprite": "minecraft:block/redstone_dust_line0", "force_translucent": true}
        });
        let (texture, forced) =
            resolve_texture_material("#line", textures.as_object().unwrap(), "minecraft").unwrap();
        assert_eq!(texture.key(), "minecraft:block/redstone_dust_line0");
        assert!(forced);
    }
    #[test]
    fn generated_item_uses_sprite_edges_and_inherited_first_person_display() {
        let temp = crate::test_dir::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("pack.mcmeta"),
            r#"{"pack":{"min_format":[97,1],"max_format":[97,1]}}"#,
        )
        .unwrap();
        for (name, contents) in [
            (
                "assets/test/items/pane.json",
                r#"{"model":{"type":"minecraft:model","model":"test:item/pane"}}"#,
            ),
            (
                "assets/test/models/item/pane.json",
                r#"{"parent":"test:item/generated","textures":{"layer0":"test:item/pane"}}"#,
            ),
            (
                "assets/test/models/item/generated.json",
                r#"{"parent":"builtin/generated","display":{"firstperson_righthand":{"rotation":[0,-90,25],"translation":[1.13,3.2,1.13],"scale":[0.68,0.68,0.68]}}}"#,
            ),
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let sprite_path = root.join("assets/test/textures/item/pane.png");
        fs::create_dir_all(sprite_path.parent().unwrap()).unwrap();
        let mut sprite = RgbaImage::new(16, 16);
        sprite.put_pixel(4, 5, Rgba([255, 255, 255, 255]));
        sprite.save(sprite_path).unwrap();
        let pack = PackStack::open(vec![root.into()]).unwrap();
        let id = ResourceId::parse("test:pane").unwrap();
        let model = resolve_item_model(&pack, &id).unwrap().unwrap();
        assert_eq!(model.elements.len(), 5); // front, back, and four exposed edges
        assert_eq!(model.elements[0].faces.len(), 2);
        // ItemModelGenerator's LEFT and RIGHT side names use EAST and WEST
        // FaceInfo respectively, with Y running from top to bottom.
        let left = &model.elements[3];
        assert_eq!(left.faces[0].direction, "east");
        assert_eq!(left.from, [4.0 / 16.0, 11.0 / 16.0, 7.5 / 16.0]);
        assert_eq!(left.to, [4.0 / 16.0, 10.0 / 16.0, 8.5 / 16.0]);
        let right = &model.elements[4];
        assert_eq!(right.faces[0].direction, "west");
        assert_eq!(right.from, [5.0 / 16.0, 11.0 / 16.0, 7.5 / 16.0]);
        assert_eq!(right.to, [5.0 / 16.0, 10.0 / 16.0, 8.5 / 16.0]);
        let transform = item_first_person_transform(&pack, &id).unwrap();
        let center = transform.transform_point3(Vec3::splat(0.5));
        assert!((center.x - 1.13 / 16.0).abs() < 1e-5);
        assert!((center.y - 3.2 / 16.0).abs() < 1e-5);
        assert!((center.z - 1.13 / 16.0).abs() < 1e-5);
    }
    #[test]
    fn double_chest_halves_use_entity_textures_and_hide_join_faces() {
        for (kind, texture, hidden, x0, x1) in [
            ("right", "normal_right", "east", 1.0 / 16.0, 1.0),
            ("left", "normal_left", "west", 0.0, 15.0 / 16.0),
        ] {
            let model =
                closed_chest_model(&Block::new("minecraft:chest").with("type", kind)).unwrap();
            assert_eq!(model.elements[0].from[0], x0);
            assert_eq!(model.elements[0].to[0], x1);
            assert!(model
                .elements
                .iter()
                .all(|part| part.faces.iter().all(|face| {
                    face.direction != hidden && face.texture.path.ends_with(texture)
                })));
            let lid_south = model.elements[1]
                .faces
                .iter()
                .find(|face| face.direction == "south")
                .unwrap();
            assert_eq!(lid_south.uv[1], 19.0 / 64.0);
            assert_eq!(lid_south.uv[3], 14.0 / 64.0);
            let lock_south = model.elements[2]
                .faces
                .iter()
                .find(|face| face.direction == "south")
                .unwrap();
            assert_eq!(lock_south.uv[1], 5.0 / 64.0);
            assert_eq!(lock_south.uv[3], 1.0 / 64.0);
        }
    }
    #[test]
    fn variant_and_multipart() {
        let state: Value =
            serde_json::from_str(r#"{"variants":{"axis=x":{"model":"x"},"axis=y":{"model":"y"}}}"#)
                .unwrap();
        let props = BTreeMap::from([("axis".into(), "y".into())]);
        assert_eq!(choose_models(&state, &props).unwrap()[0]["model"], "y");
        let multi:Value=serde_json::from_str(r#"{"multipart":[{"apply":{"model":"a"}},{"when":{"axis":"x"},"apply":{"model":"b"}}]}"#).unwrap();
        assert_eq!(choose_models(&multi, &props).unwrap().len(), 1);
    }
    #[test]
    fn omitted_uv_uses_element_bounds_and_stays_in_tile() {
        let temp = crate::test_dir::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("pack.mcmeta"),
            r#"{"pack":{"min_format":[97,1],"max_format":[97,1]}}"#,
        )
        .unwrap();
        let state = root.join("assets/test/blockstates/sample.json");
        fs::create_dir_all(state.parent().unwrap()).unwrap();
        fs::write(state, r#"{"variants":{"":{"model":"test:block/sample"}}}"#).unwrap();
        let model = root.join("assets/test/models/block/sample.json");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(model, r##"{"textures":{"all":"test:block/stone"},"elements":[{"from":[2,4,6],"to":[14,12,10],"faces":{"down":{"texture":"#all"},"north":{"texture":"#all"},"up":{"texture":"#all"}}}]}"##).unwrap();
        let packs = PackStack::open(vec![root.into()]).unwrap();
        let result = resolve_block(&packs, &Block::new("test:sample")).unwrap();
        let faces = &result.elements[0].faces;
        let face = |name: &str| faces.iter().find(|f| f.direction == name).unwrap().uv;
        assert_eq!(face("down"), [0.125, 0.375, 0.875, 0.625]);
        assert_eq!(face("north"), [0.125, 0.25, 0.875, 0.75]);
        assert_eq!(face("up"), [0.125, 0.375, 0.875, 0.625]);
        for f in faces {
            assert!(f.uv.iter().all(|value| (0.0..=1.0).contains(value)));
        }
        assert_eq!(
            implicit_uv("south", [0.0; 3], [1.0; 3]).unwrap(),
            [0.0, 0.0, 1.0, 1.0]
        );
    }

    #[test]
    fn weighted_variants_keep_each_rotation_and_weight() {
        let temp = crate::test_dir::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("pack.mcmeta"),
            r#"{"pack":{"min_format":[97,1],"max_format":[97,1]}}"#,
        )
        .unwrap();
        let state = root.join("assets/test/blockstates/sample.json");
        fs::create_dir_all(state.parent().unwrap()).unwrap();
        fs::write(state, r#"{"variants":{"": [{"model":"test:block/sample","y":0},{"model":"test:block/sample","y":90,"weight":2},{"model":"test:block/sample","y":180},{"model":"test:block/sample","y":270}]}}"#).unwrap();
        let model = root.join("assets/test/models/block/sample.json");
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(model, r##"{"textures":{"all":"test:block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}"##).unwrap();
        let packs = PackStack::open(vec![root.into()]).unwrap();
        let variants = resolve_block_variants(&packs, &Block::new("test:sample")).unwrap();
        assert_eq!(
            variants
                .iter()
                .map(|(model, weight)| (model.elements[0].rotation_y, *weight))
                .collect::<Vec<_>>(),
            vec![(0, 1), (90, 2), (180, 1), (270, 1)]
        );
    }
}

fn end_portal_model()->Result<ResolvedModel>{
    Ok(ResolvedModel{elements:vec![Element{from:[0.,0.,0.],to:[1.,0.75,1.],faces:vec![Face{direction:"up".into(),texture:ResourceId::parse("minecraft:entity/end_portal/end_portal")?,uv:[0.,0.,16.,16.],cull:false,cullface:None,tint:false,tint_index:None,force_translucent:false}],rotation_y:0,rotation:None,shade_direction_override:None}]})
}
