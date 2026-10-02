//! The one material used for map geometry and props.
//!
//! Covers Source's `LightmappedGeneric`, `WorldVertexTransition` (two
//! textures blended by vertex alpha, sharpened by `$blendmodulatetexture`),
//! `UnlitGeneric`, and `VertexLitGeneric` props lit by the map's ambient
//! cube plus the sun.

use bevy::asset::Asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

pub const SHADER_PATH: &str = "embedded://client/shaders/world.wgsl";

pub mod flags {
    pub const ALPHA_TEST: u32 = 1 << 0;
    /// Blend `$basetexture2` by vertex alpha.
    pub const BLEND: u32 = 1 << 1;
    pub const LIGHTMAPPED: u32 = 1 << 2;
    pub const UNLIT: u32 = 1 << 3;
    pub const BLEND_MODULATE: u32 = 1 << 4;
    /// Lighting baked into vertex colours (static props).
    pub const VERTEX_LIT: u32 = 1 << 5;
    pub const TRANSLUCENT: u32 = 1 << 6;
    /// Use the base texture's alpha as blend weight (`$basealphaenvmapmask` etc. ignored).
    pub const ADDITIVE: u32 = 1 << 7;
    /// Lit per frame from the material's ambient cube and sun (characters, viewmodel).
    pub const DYNAMIC_LIT: u32 = 1 << 8;
    pub const NORMAL: u32 = 1 << 9;
    pub const NORMAL2: u32 = 1 << 10;
    pub const SSBUMP: u32 = 1 << 11;
    pub const SSBUMP_MATHFIX: u32 = 1 << 12;
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct WorldParams {
    /// RGB tint, A = alpha.
    pub color: Vec4,
    pub flags: u32,
    pub alpha_ref: f32,
    pub exposure: f32,
    pub lightmap_scale: f32,
    /// Dynamic lighting: +X, -X, +Y, -Y, +Z, -Z (Bevy axes).
    pub ambient_cube: [Vec4; 6],
    /// xyz: direction towards the sun (Bevy axes); w: sun visibility 0..1.
    pub sun_dir: Vec4,
    pub sun_color: Vec4,
    /// Viewmodel optic aperture: physical screen centre xy and radius z. Zero disables.
    pub scope_aperture: Vec4,
    /// Phong exponent, boost, exponent-map enabled, base-alpha specular mask.
    pub phong: Vec4,
    pub phong_tint: Vec4,
    pub fresnel: Vec4,
    /// Rim exponent, boost, exponent-alpha mask, half-Lambert.
    pub rim: Vec4,
    /// Cosmetic style, wear, rotation cosine/sine. Zero style disables.
    pub skin: Vec4,
    pub skin_transform: Vec4,
    pub skin_colors: [Vec4; 8],
    pub skin_details:Vec4,
    pub skin_properties:Vec4,
    pub skin_pattern_palette:Vec4,
}

impl Default for WorldParams {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            flags: 0,
            alpha_ref: 0.5,
            exposure: 1.0,
            lightmap_scale: 1.0,
            ambient_cube: [Vec4::splat(0.4); 6],
            sun_dir: Vec4::new(0.0, 1.0, 0.0, 0.0),
            sun_color: Vec4::ZERO,
            scope_aperture: Vec4::ZERO,
            phong: Vec4::new(20.0,0.0,0.0,0.0),
            phong_tint: Vec4::ONE,
            fresnel: Vec4::new(0.0,0.5,1.0,0.0),
            rim: Vec4::ZERO,
            skin: Vec4::ZERO,
            skin_transform: Vec4::new(1.0,1.0,0.0,0.0),
            skin_colors: [Vec4::ONE;8],
            skin_details:Vec4::ZERO,
            skin_properties:Vec4::ZERO,
            skin_pattern_palette:Vec4::new(1.,2.,3.,4.),
        }
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldMaterialKey {
    pub cull: bool,
    pub depth_write: bool,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(WorldMaterialKey)]
pub struct WorldMaterial {
    #[uniform(0)]
    pub params: WorldParams,
    #[texture(1)]
    #[sampler(2)]
    pub base: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub base2: Handle<Image>,
    #[texture(5, dimension = "2d_array")]
    #[sampler(6)]
    pub lightmap: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    pub modulate: Handle<Image>,
    #[texture(9)]
    #[sampler(10)]
    pub normal: Handle<Image>,
    #[texture(11)]
    #[sampler(12)]
    pub normal2: Handle<Image>,
    /// Eight texels per static prop: ambient cube, sun direction, sun color.
    #[texture(13)]
    pub prop_light: Handle<Image>,
    #[texture(14)]
    #[sampler(15)]
    pub phong_map: Handle<Image>,
    #[texture(16)]
    #[sampler(17)]
    pub skin_surface:Handle<Image>,
    #[texture(18)]
    #[sampler(19)]
    pub skin_position:Handle<Image>,
    pub alpha_mode: AlphaMode,
    pub cull: bool,
}

impl From<&WorldMaterial> for WorldMaterialKey {
    fn from(m: &WorldMaterial) -> Self {
        Self {
            cull: m.cull,
            depth_write: !matches!(m.alpha_mode, AlphaMode::Blend | AlphaMode::Add | AlphaMode::Premultiplied),
        }
    }
}

impl Material for WorldMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = if key.bind_group_data.cull { Some(Face::Back) } else { None };
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_write_enabled = Some(key.bind_group_data.depth_write);
        }
        Ok(())
    }
}
