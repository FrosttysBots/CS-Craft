#import bevy_pbr::{forward_io::Vertex, mesh_functions, view_transformations::position_world_to_clip}
#import bevy_pbr::mesh_view_bindings::view
// Centroid UVs stay inside a covered triangle at MSAA silhouette/quad edges.
// Center interpolation can sample an adjacent atlas sprite outside the face.
struct VoxelFragment {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(2) @interpolate(perspective, centroid) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
    @location(5) color: vec4<f32>,
};
@vertex fn vertex(input: Vertex) -> VoxelFragment {
    var out: VoxelFragment;
    let transform = mesh_functions::get_world_from_local(input.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(transform, vec4<f32>(input.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.uv = input.uv;
    out.uv_b = input.uv_b;
    out.color = input.color;
    return out;
}
struct Parameters { daylight:vec4<f32>, fog:vec4<f32> };
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params:Parameters;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var atlas_sampler:sampler;
@fragment fn fragment(in:VoxelFragment)->@location(0) vec4<f32>{
    let tex=textureSample(atlas,atlas_sampler,in.uv);
    let blended=params.daylight.w>0.5;
    if tex.a < select(0.5,0.01,blended) {discard;}
    // Destruction overlays multiply the already-lit block; transparent texels
    // contribute white and never brighten the underlying surface.
    if params.daylight.w > 1.5 {
        return vec4<f32>(mix(vec3<f32>(1.0),tex.rgb,tex.a*in.color.a),1.0);
    }
    let light=max(vec3<f32>(max(0.025,params.daylight.z)),vec3<f32>(0.86,0.92,1.0)*pow(in.uv_b.x,1.6)*params.daylight.x+vec3<f32>(1.0,0.76,0.48)*pow(in.uv_b.y,1.6));
    var surface=tex.rgb*in.color.rgb;
    // Native LivingEntity overlay: red mixed with the sampled skin for ten ticks.
    // Its negative alpha is metadata, so it must never make the mob transparent.
    if in.color.a < 0.0 { surface=mix(vec3<f32>(1.0,0.0,0.0),surface,clamp(-in.color.a,0.0,1.0)); }
    let color=surface*light;
    let distance=length(in.world_position.xyz-view.world_position);
    let fog=smoothstep(params.daylight.y*0.65,params.daylight.y,distance);
    return vec4<f32>(mix(color,params.fog.rgb,fog),select(1.0,tex.a*in.color.a,blended));
}
