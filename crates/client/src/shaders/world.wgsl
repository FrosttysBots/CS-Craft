// World / prop shader for CSGO Rewrite.
//
// Inputs come from Bevy's default mesh vertex shader:
//   uv    = base texture coordinates
//   uv_b  = lightmap atlas coordinates (world geometry)
//   color = rgb: vertex tint, a: WorldVertexTransition blend weight

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view

struct WorldParams {
    color: vec4<f32>,
    flags: u32,
    alpha_ref: f32,
    exposure: f32,
    lightmap_scale: f32,
    ambient_cube: array<vec4<f32>, 6>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    scope_aperture: vec4<f32>,
    phong: vec4<f32>,
    phong_tint: vec4<f32>,
    fresnel: vec4<f32>,
    rim: vec4<f32>,
    skin: vec4<f32>,
    skin_transform: vec4<f32>,
    skin_colors: array<vec4<f32>,8>,
    skin_details:vec4<f32>,
    skin_properties:vec4<f32>,
    skin_pattern_palette:vec4<f32>,
};


const ALPHA_TEST: u32 = 1u;
const BLEND: u32 = 2u;
const LIGHTMAPPED: u32 = 4u;
const UNLIT: u32 = 8u;
const BLEND_MODULATE: u32 = 16u;
const VERTEX_LIT: u32 = 32u;
const TRANSLUCENT: u32 = 64u;
const DYNAMIC_LIT: u32 = 256u;
const NORMAL: u32 = 512u;
const NORMAL2: u32 = 1024u;
const SSBUMP: u32 = 2048u;
const SSBUMP_MATHFIX: u32 = 4096u;
// Orthonormal radiosity basis, in texture tangent space.
const RNM0 = vec3<f32>(0.81649658, 0.0, 0.57735027);
const RNM1 = vec3<f32>(-0.40824829, 0.70710678, 0.57735027);
const RNM2 = vec3<f32>(-0.40824829, -0.70710678, 0.57735027);

fn ambient_light(n: vec3<f32>) -> vec3<f32> {
    let n2 = n * n;
    var c = vec3<f32>(0.0);
    c += n2.x * select(params.ambient_cube[1].rgb, params.ambient_cube[0].rgb, n.x >= 0.0);
    c += n2.y * select(params.ambient_cube[3].rgb, params.ambient_cube[2].rgb, n.y >= 0.0);
    c += n2.z * select(params.ambient_cube[5].rgb, params.ambient_cube[4].rgb, n.z >= 0.0);
    return c;
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WorldParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var base2_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var base2_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var light_tex: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var light_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var mod_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var mod_samp: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(9) var normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var normal_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var normal2_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(12) var normal2_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(13) var prop_light: texture_2d<f32>;

@group(#{MATERIAL_BIND_GROUP}) @binding(14) var phong_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(15) var phong_samp: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(16) var skin_surface: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(17) var skin_surface_samp: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(18) var skin_position: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(19) var skin_position_samp: sampler;
fn paint_coords(uv:vec2<f32>)->vec2<f32>{
    let p=(uv-vec2<f32>(0.5))*params.skin_transform.xy;
    return vec2<f32>(p.x*params.skin.z-p.y*params.skin.w,p.x*params.skin.w+p.y*params.skin.z)+vec2<f32>(0.5)+params.skin_transform.zw;
}
fn rnm_weights(n: vec3<f32>) -> vec3<f32> {
    let d = max(vec3<f32>(dot(n, RNM0), dot(n, RNM1), dot(n, RNM2)), vec3<f32>(0.0));
    let w = d * d;
    return w / max(dot(w, vec3<f32>(1.0)), 0.00001);
}
fn bump_normal(sample: vec3<f32>) -> vec3<f32> {
    if (params.flags & SSBUMP) != 0u {
        return normalize(RNM0 * sample.x + RNM1 * sample.y + RNM2 * sample.z);
    }
    return normalize(sample * 2.0 - 1.0);
}
fn bump_weights(sample: vec3<f32>) -> vec3<f32> {
    // SSBump stores precomputed directional visibility; normalization would
    // erase its self-shadowing. The basis' constant Z converts to irradiance.
    if (params.flags & SSBUMP) != 0u { return sample * select(1.0, 0.57735027, (params.flags & SSBUMP_MATHFIX) != 0u); }
    return rnm_weights(bump_normal(sample));
}
fn prop_irradiance(n: vec3<f32>, id: i32) -> vec3<f32> {
    let n2 = n * n;
    let ix = select(1, 0, n.x >= 0.0);
    let iy = select(3, 2, n.y >= 0.0);
    let iz = select(5, 4, n.z >= 0.0);
    let ambient = n2.x * textureLoad(prop_light, vec2<i32>(ix, id), 0).rgb
        + n2.y * textureLoad(prop_light, vec2<i32>(iy, id), 0).rgb
        + n2.z * textureLoad(prop_light, vec2<i32>(iz, id), 0).rgb;
    let sun = textureLoad(prop_light, vec2<i32>(7, id), 0).rgb;
    let dir = textureLoad(prop_light, vec2<i32>(6, id), 0).xyz;
    return ambient + sun * max(dot(n, dir), 0.0);
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
    if params.skin_details.w > 0.5 && params.skin_details.z > 0.0 {
        let grain=fract(sin(dot(floor(uv*512.0),vec2<f32>(12.9898,78.233)))*43758.5453);
        albedo.a*=smoothstep(params.skin_details.z-0.05,params.skin_details.z+0.1,grain);
    }
    // Weapon cosmetics reuse the otherwise inactive terrain-blend bindings.
    // Keep alpha and normal/specular maps from the original weapon material.
    if params.skin.x > 0.5 {
        let style = u32(params.skin.x);
        let mask = textureSample(mod_tex, mod_samp, uv);
        let centered = (uv - vec2<f32>(0.5)) * params.skin_transform.xy;
        let rotated = vec2<f32>(centered.x*params.skin.z-centered.y*params.skin.w,
                               centered.x*params.skin.w+centered.y*params.skin.z);
        let paint_uv = rotated + vec2<f32>(0.5) + params.skin_transform.zw;
        var pattern = textureSample(base2_tex, base2_samp, paint_uv);
        if (style==3u || style==6u) && params.skin_properties.y>0.5 {
            let pos=textureSample(skin_position,skin_position_samp,uv).rgb;
            let normal=abs(textureSample(skin_surface,skin_surface_samp,uv).rgb*2.0-1.0);
            let weights=pow(normal,vec3<f32>(4.0))/max(dot(pow(normal,vec3<f32>(4.0)),vec3<f32>(1.0)),0.00001);
            pattern=textureSample(base2_tex,base2_samp,paint_coords(pos.yz))*weights.x
                +textureSample(base2_tex,base2_samp,paint_coords(pos.xz))*weights.y
                +textureSample(base2_tex,base2_samp,paint_coords(pos.xy))*weights.z;
        }
        var paint = pattern.rgb;
        if style <= 6u {
            var weights = pattern.rgb;
            if style == 1u || style == 4u { weights = mask.rgb; }
            paint = params.skin_colors[0].rgb;
            paint = mix(paint,params.skin_colors[1].rgb,weights.r);
            paint = mix(paint,params.skin_colors[2].rgb,weights.g);
            paint = mix(paint,params.skin_colors[3].rgb,weights.b);
        }
        if style == 10u {
            // Installed glove masks store the palette index in alpha;
            // RGB selects fabric/leather material properties, not colors.
            let index=min(u32(mask.a*8.0),7u);
            paint=params.skin_colors[index].rgb;
            if index+1u==u32(params.skin_details.x){
                let indices=vec4<u32>(clamp(params.skin_pattern_palette-vec4<f32>(1.0),vec4<f32>(0.0),vec4<f32>(7.0)));
                paint=params.skin_colors[indices.x].rgb;
                paint=mix(paint,params.skin_colors[indices.y].rgb,pattern.r);
                paint=mix(paint,params.skin_colors[indices.z].rgb,pattern.g);
                paint=mix(paint,params.skin_colors[indices.w].rgb,pattern.b);
            }
        }
        if style>=4u && style<=6u {paint*=max(params.skin_properties.x,1.0);}
        let authored=textureSample(normal2_tex, normal2_samp, uv);
        let ao=authored.r;
        // Authored surface/grunge maps drive edge wear; UV noise is only the fallback.
        let grain = fract(sin(dot(floor(paint_uv*1024.0),vec2<f32>(12.9898,78.233)))*43758.5453);
        let wear = params.skin.y * 0.65;
        let surface=textureSample(skin_surface,skin_surface_samp,uv);
        let damage=select(mask.a*0.55+grain*0.45,select(authored.g*0.65+grain*0.35,surface.r,style==10u),params.skin_details.y>0.5);
        let exposed = smoothstep(1.0-wear,1.0,damage);
        if style == 10u {albedo=vec4<f32>(paint*0.45,albedo.a);}
        var coverage = 1.0-exposed*wear;
        if style != 7u && style != 9u && style != 10u {coverage *= clamp(mask.r+mask.g+mask.b,0.0,1.0);}
        if style>=4u && style<=6u {coverage*=mask.r*(1.0-mask.g);}
        if style==10u {coverage=1.0-exposed*wear;}
        albedo = vec4<f32>(mix(albedo.rgb,paint*mix(0.55,1.0,ao),coverage),albedo.a);
    }

#ifdef VERTEX_COLORS
    let vcolor = in.color;
#else
    let vcolor = vec4<f32>(1.0);
#endif

    var blend_weight = 0.0;
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
        blend_weight = t;
        albedo = mix(albedo, second, t);
    }

    albedo = vec4<f32>(albedo.rgb * params.color.rgb, albedo.a * params.color.a);

    if (params.flags & ALPHA_TEST) != 0u && albedo.a < params.alpha_ref {
        discard;
    }

    var n = normalize(in.world_normal);
    var weights = vec3<f32>(1.0 / 3.0);
    if (params.flags & NORMAL) != 0u {
        let b1 = textureSample(normal_tex, normal_samp, uv).rgb;
        var tangent_normal = bump_normal(b1);
        weights = bump_weights(b1);
        if (params.flags & NORMAL2) != 0u {
            let b2 = textureSample(normal2_tex, normal2_samp, uv).rgb;
            tangent_normal = normalize(mix(tangent_normal, bump_normal(b2), blend_weight));
            weights = mix(weights, bump_weights(b2), blend_weight);
        }
#ifdef VERTEX_TANGENTS
        let t = normalize(in.world_tangent.xyz - n * dot(n, in.world_tangent.xyz));
        let b = cross(n, t) * in.world_tangent.w;
        n = normalize(t * tangent_normal.x + b * tangent_normal.y + n * tangent_normal.z);
#endif
    }
    var light = vec3<f32>(1.0);
    if (params.flags & LIGHTMAPPED) != 0u {
#ifdef VERTEX_UVS_B
        light = textureSample(light_tex, light_samp, in.uv_b, 0).rgb;
        if (params.flags & NORMAL) != 0u {
            light = textureSample(light_tex, light_samp, in.uv_b, 1).rgb * weights.x
                + textureSample(light_tex, light_samp, in.uv_b, 2).rgb * weights.y
                + textureSample(light_tex, light_samp, in.uv_b, 3).rgb * weights.z;
        }
        light *= params.lightmap_scale;
#endif
    } else if (params.flags & VERTEX_LIT) != 0u {
        // Static props: per-vertex lighting baked at load time.
        light = vcolor.rgb;
#ifdef VERTEX_UVS_B
        if (params.flags & NORMAL) != 0u { light = prop_irradiance(n, i32(round(in.uv_b.x))); }
#endif
    } else if (params.flags & DYNAMIC_LIT) != 0u {
        let ndl=dot(n, params.sun_dir.xyz);
        let diffuse=select(max(ndl,0.0),pow(ndl*0.5+0.5,2.0),params.rim.w>0.5);
        light = ambient_light(n) + params.sun_color.rgb * diffuse * params.sun_dir.w;
    }
    if (params.flags & UNLIT) != 0u {
        light = vec3<f32>(1.0);
    }

    if params.skin_details.w>0.5 && params.skin_properties.z>0.5 {
        let eye=normalize(view.world_position.xyz-in.world_position.xyz);
        let mask=textureSample(mod_tex,mod_samp,uv);
        let angle=clamp(1.0-abs(dot(n,eye)),0.0,1.0);
        let spectrum=textureSample(base2_tex,base2_samp,vec2<f32>(fract(angle+mask.g),mask.b));
        albedo=vec4<f32>(mix(albedo.rgb,albedo.rgb*spectrum.rgb*2.0,mask.r),albedo.a);
    }
    var rgb = albedo.rgb * light;
    if (params.flags & DYNAMIC_LIT) != 0u && params.phong.y > 0.0 {
        let v=normalize(view.world_position.xyz-in.world_position.xyz);
        let nv=clamp(dot(n,v),0.0,1.0);
        let fresnel_t=1.0-nv;
        let fresnel=select(mix(params.fresnel.x,params.fresnel.y,fresnel_t*2.0),mix(params.fresnel.y,params.fresnel.z,(fresnel_t-0.5)*2.0),fresnel_t>0.5);
        let exponent_sample=textureSample(phong_tex,phong_samp,uv);
        let exponent=select(params.phong.x,1.0+exponent_sample.r*149.0,params.phong.z>0.5);
        let normal_alpha=textureSample(normal_tex,normal_samp,uv).a;
        let mask=select(select(1.0,normal_alpha,(params.flags & NORMAL)!=0u),albedo.a,params.phong.w>0.5);
        let tint=params.phong_tint.rgb*mix(vec3<f32>(1.0),albedo.rgb,params.phong_tint.w*exponent_sample.g);
        let reflected=reflect(-params.sun_dir.xyz,n);
        let spec=pow(max(dot(reflected,v),0.0),exponent)*params.phong.y*fresnel*mask;
        rgb+=tint*params.sun_color.rgb*params.sun_dir.w*spec*select(0.0,1.0,dot(n,params.sun_dir.xyz)>0.0);
        let rim_mask=select(1.0,exponent_sample.a,params.rim.z>0.5 && params.phong.z>0.5);
        rgb+=ambient_light(v)*tint*pow(1.0-nv,max(params.rim.x,1.0))*params.rim.y*rim_mask*0.25;
    }
    if (params.flags & UNLIT) == 0u {
        rgb = tonemap(rgb * params.exposure);
    }
    var alpha = 1.0;
    if (params.flags & TRANSLUCENT) != 0u {
        alpha = albedo.a;
    }
    // A magnifying optic reveals the world through its rear lens rather
    // than the unmagnified gun barrel. Only held-weapon materials enable this.
    if params.scope_aperture.z > 0.0 && distance(in.position.xy, params.scope_aperture.xy) < params.scope_aperture.z {
        discard;
    }
    return vec4<f32>(rgb, alpha);
}
