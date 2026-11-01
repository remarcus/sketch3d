struct Scene {
    view_projection: mat4x4<f32>, light_projection: mat4x4<f32>,
    light: vec4<f32>, settings: vec4<f32>, shadow: vec4<f32>,
}
@group(0) @binding(0) var<uniform> scene: Scene;
@group(1) @binding(0) var brush_mask: texture_2d<f32>;
@group(1) @binding(1) var brush_sampler: sampler;
struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) material: f32, }
@vertex fn vs_shadow(@location(0) position: vec3<f32>, @location(3) uv: vec2<f32>, @location(5) material: f32) -> Output {
    var out: Output;
    out.position=scene.light_projection*vec4<f32>(position,1.0); out.uv=uv; out.material=material; return out;
}
@fragment fn fs_shadow(in: Output) {
    if in.material>0.5 { discard; }
    if textureSampleLevel(brush_mask,brush_sampler,in.uv,0.0).a<0.5 { discard; }
}
