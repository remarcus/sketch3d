struct Camera { view_projection: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
}
@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>) -> Output {
    var out: Output;
    out.position = camera.view_projection * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.color = color;
    return out;
}
@fragment fn fs_main(in: Output) -> @location(0) vec4<f32> {
    if length(in.normal) < 0.1 { return vec4<f32>(in.color, 1.0); }
    let light = normalize(vec3<f32>(0.4, 0.7, 1.0));
    let shade = 0.55 + 0.45 * abs(dot(normalize(in.normal), light));
    return vec4<f32>(in.color * shade, 1.0);
}
