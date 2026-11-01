struct SceneUniform {
    view: mat4x4<f32>,
    light_view: mat4x4<f32>,
    light: vec4<f32>,
    settings: vec4<f32>,
    shadow: vec4<f32>,
    camera: vec4<f32>,
};

@group(0) @binding(0) var<uniform> scene: SceneUniform;
@group(1) @binding(0) var image_texture: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
fn apply_grain(color: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    let amount = scene.camera.w;
    if amount <= 0.0 { return color; }
    let pixel = floor(position);
    let random = fract(sin(dot(pixel, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    return clamp(color + vec3<f32>((random * 2.0 - 1.0) * amount), vec3<f32>(0.0), vec3<f32>(1.0));
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) opacity: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
};

@vertex
fn vs_image(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = scene.view * vec4<f32>(input.position, 1.0);
    output.uv = input.uv;
    output.opacity = input.opacity;
    return output;
}

@fragment
fn fs_image(input: VertexOutput) -> FragmentOutput {
    var color = textureSample(image_texture, image_sampler, input.uv);
    color = vec4<f32>(apply_grain(color.rgb, input.clip_position.xy), color.a);
    color.a *= input.opacity;
    var output: FragmentOutput;
    output.color = color;
    output.emission = vec4<f32>(0.0);
    return output;
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @location(1) emission: vec4<f32>,
}
