#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
struct Settings { intensity: f32, }
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: Settings;
@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let step=settings.intensity / vec2<f32>(textureDimensions(scene));
    var color=vec3<f32>(0.0);
    var weight=0.0;
    for(var y=-3;y<=3;y++) {
        for(var x=-3;x<=3;x++) {
            let w=exp(-f32(x*x+y*y)/5.0);
            color+=textureSample(scene,scene_sampler,in.uv+vec2<f32>(f32(x),f32(y))*step).rgb*w;
            weight+=w;
        }
    }
    return vec4<f32>(color/weight,1.0);
}
