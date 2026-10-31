struct Scene {
    view_projection: mat4x4<f32>, light_projection: mat4x4<f32>,
    light: vec4<f32>, settings: vec4<f32>, shadow: vec4<f32>,
}
@group(0) @binding(0) var<uniform> scene: Scene;
@group(1) @binding(0) var brush_mask: texture_2d<f32>;
@group(1) @binding(1) var brush_sampler: sampler;
@group(2) @binding(0) var shadow_depth: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;
struct Output {
    @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) world: vec3<f32>,
}
@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>, @location(3) uv: vec2<f32>) -> Output {
    var out: Output;
    out.position=scene.view_projection*vec4<f32>(position,1.0);
    out.normal=normal; out.color=color; out.uv=uv; out.world=position; return out;
}
fn visibility(world: vec3<f32>, normal: vec3<f32>) -> f32 {
    if scene.settings.y<0.5 { return 1.0; }
    let oriented=select(-normal,normal,dot(normal,scene.light.xyz)>=0.0);
    let clip=scene.light_projection*vec4<f32>(world+oriented*scene.shadow.y,1.0);
    let p=clip.xyz/clip.w;
    let uv=vec2<f32>(p.x*0.5+0.5,0.5-p.y*0.5);
    if any(uv<vec2<f32>(0.0)) || any(uv>vec2<f32>(1.0)) || p.z<0.0 || p.z>1.0 { return 1.0; }
    var sum=0.0;
    for (var y=-1; y<=1; y+=1) {
        for (var x=-1; x<=1; x+=1) {
            sum+=textureSampleCompareLevel(shadow_depth,shadow_sampler,uv+vec2<f32>(f32(x),f32(y))*scene.shadow.x,p.z-scene.settings.w);
        }
    }
    return mix(1.0,sum/9.0,scene.settings.z);
}
@fragment fn fs_main(in: Output) -> @location(0) vec4<f32> {
    if length(in.normal)<0.1 { return vec4<f32>(in.color,1.0); }
    let mask_alpha=textureSampleLevel(brush_mask,brush_sampler,in.uv,0.0).a;
    var coverage=mask_alpha;
    if scene.shadow.z<0.5 {
        if mask_alpha<0.5 { discard; }
        coverage=1.0;
    }
    let normal=normalize(in.normal);
    let shade=scene.light.w+scene.settings.x*abs(dot(normal,scene.light.xyz))*visibility(in.world,normal);
    return vec4<f32>(in.color*shade,coverage);
}
