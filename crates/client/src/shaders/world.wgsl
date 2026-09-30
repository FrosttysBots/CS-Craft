// World / prop shader for CSGO Rewrite.
//
// Inputs come from Bevy's default mesh vertex shader:
//   uv    = base texture coordinates
//   uv_b  = lightmap atlas coordinates (world geometry)
//   color = rgb: vertex tint, a: WorldVertexTransition blend weight

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_functions::get_tag

struct WorldParams {
    color: vec4<f32>,
    flags: u32,
    alpha_ref: f32,
    exposure: f32,
    lightmap_scale: f32,
};

struct PropLight {
    ambient_cube: array<vec4<f32>, 6>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
};

const ALPHA_TEST: u32 = 1u;
const BLEND: u32 = 2u;
const LIGHTMAPPED: u32 = 4u;
const UNLIT: u32 = 8u;
const BLEND_MODULATE: u32 = 16u;
const VERTEX_LIT: u32 = 32u;
const TRANSLUCENT: u32 = 64u;

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WorldParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var base2_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var base2_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var light_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var light_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var mod_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var mod_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var<storage, read> prop_lights: array<PropLight>;

fn ambient_light(pl: PropLight, n: vec3<f32>) -> vec3<f32> {
    let n2 = n * n;
    var c = vec3<f32>(0.0);
    c += n2.x * select(pl.ambient_cube[1].rgb, pl.ambient_cube[0].rgb, n.x >= 0.0);
    c += n2.y * select(pl.ambient_cube[3].rgb, pl.ambient_cube[2].rgb, n.y >= 0.0);
    c += n2.z * select(pl.ambient_cube[5].rgb, pl.ambient_cube[4].rgb, n.z >= 0.0);
    return c;
}

// Extended Reinhard on luminance: keeps shade readable and rolls sunlit
// highlights off towards a white point, preserving hue.
const WHITE_POINT: f32 = 4.0;
fn tonemap(x: vec3<f32>) -> vec3<f32> {
    let l = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
    if l <= 0.0 {
        return vec3<f32>(0.0);
    }
    let mapped = l * (1.0 + l / (WHITE_POINT * WHITE_POINT)) / (1.0 + l);
    return clamp(x * (mapped / l), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
#ifdef VERTEX_UVS_A
    let uv = in.uv;
#else
    let uv = vec2<f32>(0.0);
#endif
    var albedo = textureSample(base_tex, base_samp, uv);

#ifdef VERTEX_COLORS
    let vcolor = in.color;
#else
    let vcolor = vec4<f32>(1.0);
#endif

    if (params.flags & BLEND) != 0u {
        let second = textureSample(base2_tex, base2_samp, uv);
        var t = vcolor.a;
        if (params.flags & BLEND_MODULATE) != 0u {
            // Blend modulation: G = blend centre, R = softness.
            let m = textureSample(mod_tex, mod_samp, uv);
            let lo = clamp(m.g - m.r, 0.0, 1.0);
            let hi = clamp(m.g + m.r, 0.0, 1.0);
            t = smoothstep(lo, hi + 0.0001, t);
        }
        albedo = mix(albedo, second, t);
    }

    albedo = vec4<f32>(albedo.rgb * params.color.rgb, albedo.a * params.color.a);

    if (params.flags & ALPHA_TEST) != 0u && albedo.a < params.alpha_ref {
        discard;
    }

    var light = vec3<f32>(1.0);
    if (params.flags & LIGHTMAPPED) != 0u {
#ifdef VERTEX_UVS_B
        light = textureSample(light_tex, light_samp, in.uv_b).rgb * params.lightmap_scale;
#endif
    } else if (params.flags & VERTEX_LIT) != 0u {
        let count = arrayLength(&prop_lights);
        let index = min(get_tag(in.instance_index), count - 1u);
        let pl = prop_lights[index];
        let n = normalize(in.world_normal);
        light = ambient_light(pl, n);
        let ndl = max(dot(n, pl.sun_dir.xyz), 0.0);
        light += pl.sun_color.rgb * ndl * pl.sun_dir.w;
    }
    if (params.flags & UNLIT) != 0u {
        light = vec3<f32>(1.0);
    }

    var rgb = albedo.rgb * light * vcolor.rgb;
    if (params.flags & UNLIT) == 0u {
        rgb = tonemap(rgb * params.exposure);
    }
    var alpha = 1.0;
    if (params.flags & TRANSLUCENT) != 0u {
        alpha = albedo.a;
    }
    return vec4<f32>(rgb, alpha);
}
