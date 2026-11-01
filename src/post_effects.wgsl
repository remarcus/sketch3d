struct EffectSettings {
    // glow, focus distance, depth blur strength, pixel block size
    effects: vec4<f32>,
    // camera distance, orthographic flag, near plane, far plane
    camera: vec4<f32>,
    size: vec2<f32>,
    _pad: vec2<f32>,
}
@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var<uniform> settings: EffectSettings;
@group(0) @binding(4) var emission_color: texture_2d<f32>;

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[index], 0.0, 1.0);
}

fn scene(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(scene_color, scene_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
}

fn emission(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(emission_color, scene_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
}

fn linear_depth(depth: f32) -> f32 {
    if settings.camera.y > 0.5 {
        return mix(settings.camera.z, settings.camera.w, depth);
    }
    // WebGPU's perspective depth is already in [0, 1].
    return (settings.camera.z * settings.camera.w) /
        max(settings.camera.w - depth * (settings.camera.w - settings.camera.z), 0.0001);
}

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let block = max(settings.effects.w, 1.0);
    let pixel = floor(position.xy / block) * block + vec2<f32>(block * 0.5);
    let uv = pixel / settings.size;
    var color = scene(uv);

    let blur_strength = settings.effects.z;
    if blur_strength > 0.0 && settings.effects.y > 0.0 {
        let coord = vec2<i32>(clamp(pixel, vec2<f32>(0.0), settings.size - vec2<f32>(1.0)));
        let depth = linear_depth(textureLoad(scene_depth, coord, 0));
        let radius = min(16.0, abs(depth - settings.effects.y) * blur_strength * 0.02);
        if radius > 0.25 {
            let step = vec2<f32>(radius) / settings.size;
            let blur = (scene(uv + vec2<f32>(step.x, 0.0)) + scene(uv - vec2<f32>(step.x, 0.0)) +
                        scene(uv + vec2<f32>(0.0, step.y)) + scene(uv - vec2<f32>(0.0, step.y)) +
                        scene(uv + step) + scene(uv - step) +
                        scene(uv + vec2<f32>(step.x, -step.y)) + scene(uv + vec2<f32>(-step.x, step.y))) / 8.0;
            color = mix(color, blur, clamp(radius / 8.0, 0.0, 0.82));
        }
    }

    let glow_strength = settings.effects.x;
    if glow_strength > 0.0 {
        let texel = vec2<f32>(1.0) / settings.size;
        var glow = vec3<f32>(0.0);
        for (var y = -2; y <= 2; y += 1) {
            for (var x = -2; x <= 2; x += 1) {
                glow += emission(uv + vec2<f32>(f32(x), f32(y)) * texel * 2.0);
            }
        }
        color = vec4<f32>(color.rgb + glow * (glow_strength / 25.0), color.a);
    }
    return color;
}
