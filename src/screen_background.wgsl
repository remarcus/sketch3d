@group(0) @binding(0) var image_texture: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) opacity: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
};

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @location(1) emission: vec4<f32>,
};

@vertex
fn vs_background(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = vec4<f32>(input.position, 0.999, 1.0);
    output.uv = input.uv;
    output.opacity = input.opacity;
    return output;
}

@fragment
fn fs_background(input: VertexOutput) -> FragmentOutput {
    var color = textureSample(image_texture, image_sampler, input.uv);
    color.a *= input.opacity;
    var output: FragmentOutput;
    output.color = color;
    output.emission = vec4<f32>(0.0);
    return output;
}
