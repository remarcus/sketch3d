struct Scene {
    view_projection: mat4x4<f32>, light_projection: mat4x4<f32>,
    light: vec4<f32>, settings: vec4<f32>, shadow: vec4<f32>, camera: vec4<f32>, background: vec4<f32>, resolution: vec4<f32>,
}
@group(0) @binding(0) var<uniform> scene: Scene;
@group(1) @binding(0) var brush_mask: texture_2d<f32>;
@group(1) @binding(1) var brush_sampler: sampler;
@group(2) @binding(0) var shadow_depth: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;
@group(3) @binding(0) var grain_mask: texture_2d<f32>;
@group(3) @binding(1) var grain_sampler: sampler;
@group(4) @binding(0) var clip_mask: texture_2d<f32>;
@group(4) @binding(1) var clip_sampler: sampler;
fn apply_grain(color: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    let amount = scene.camera.w;
    if amount <= 0.0 { return color; }
    let pixel = floor(position);
    let random = fract(sin(dot(pixel, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    return clamp(color + vec3<f32>((random * 2.0 - 1.0) * amount), vec3<f32>(0.0), vec3<f32>(1.0));
}
struct Output {
    @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) grain_uv: vec2<f32>,
    @location(4) world: vec3<f32>, @location(5) opacity: f32, @location(6) material: f32,
    @location(7) grain_dynamics: vec2<f32>, @location(8) emission: vec3<f32>,
}
@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>, @location(3) uv: vec2<f32>, @location(4) opacity: f32, @location(5) material: f32, @location(6) grain_uv: vec2<f32>, @location(7) grain_dynamics: vec2<f32>, @location(8) emission: vec3<f32>) -> Output {
    var out: Output;
    out.position=scene.view_projection*vec4<f32>(position,1.0);
    out.normal=normal; out.color=color; out.uv=uv; out.grain_uv=grain_uv; out.world=position; out.opacity=opacity; out.material=material; out.grain_dynamics=grain_dynamics; out.emission=emission; return out;
}
fn grain_coverage(in: Output, brush_alpha: f32) -> f32 {
    if in.material >= 4.0 {
        var grain_alpha = textureSampleLevel(grain_mask, grain_sampler, in.grain_uv, 0.0).a;
        grain_alpha = clamp((grain_alpha - 0.5) * in.grain_dynamics.y + 0.5, 0.0, 1.0);
        let grain_coverage = 1.0 - (1.0 - grain_alpha) * clamp(in.grain_dynamics.x, 0.0, 1.0);
        return brush_alpha * grain_coverage;
    }
    let alpha = clamp((brush_alpha - 0.5) * in.grain_dynamics.y + 0.5, 0.0, 1.0);
    return 1.0 - (1.0 - alpha) * clamp(in.grain_dynamics.x, 0.0, 1.0);
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
struct FragmentOutput { @location(0) color: vec4<f32>, @location(1) emission: vec4<f32>, }
fn clip_coverage(in: Output) -> f32 {
    return textureSampleLevel(clip_mask, clip_sampler, in.position.xy / max(scene.resolution.xy, vec2<f32>(1.0)), 0.0).r;
}
fn parent_clip_coverage(in: Output) -> f32 {
    let dimensions = vec2<f32>(textureDimensions(clip_mask));
    return textureSampleLevel(clip_mask, clip_sampler, in.position.xy / dimensions, 0.0).r;
}
fn material_output(in: Output, alpha: f32, sampled_color: vec3<f32>) -> FragmentOutput {
    let material = in.material - floor(in.material / 4.0) * 4.0;
    var shade=1.0;
    if material<0.5 && length(in.normal)>=0.1 {
        let normal=normalize(in.normal);
        shade=scene.light.w+scene.settings.x*abs(dot(normal,scene.light.xyz))*visibility(in.world,normal);
        if scene.shadow.w > 0.5 { shade=floor(shade*4.0+0.5)/4.0; }
    }
    var color=in.color*sampled_color*shade;
    var emission=vec3<f32>(0.0);
    if material>1.5 && material<2.5 {
        emission=in.color*sampled_color*2.0;
    }
    emission+=in.emission;
    color+=emission;
    return FragmentOutput(vec4<f32>(apply_grain(color,in.position.xy),alpha),vec4<f32>(emission*alpha,alpha));
}
@fragment fn fs_main(in: Output) -> FragmentOutput {
    let clip = clip_coverage(in);
    if clip <= 0.001 { discard; }
    if length(in.normal)<0.1 { return material_output(in, clip, vec3<f32>(1.0)); }
    let sampled_texture=textureSampleLevel(brush_mask,brush_sampler,in.uv,0.0);
    if in.grain_dynamics.y > 2.5 {
        return material_output(in, clip, sampled_texture.rgb);
    }
    if in.grain_dynamics.y > 1.5 {
        if sampled_texture.a * in.opacity < in.grain_dynamics.x { discard; }
        return material_output(in, clip, sampled_texture.rgb);
    }
    let mask_alpha=grain_coverage(in,sampled_texture.a);
    var coverage=mask_alpha;
    if scene.shadow.z<0.5 {
        if mask_alpha<0.5 { discard; }
        coverage=1.0;
    }
    return material_output(in, coverage*clip, sampled_texture.rgb);
}
@fragment fn fs_cutout(in: Output) -> FragmentOutput {
    let clip = clip_coverage(in);
    let sampled_alpha=textureSampleLevel(brush_mask,brush_sampler,in.uv,0.0).a;
    let mask_alpha=grain_coverage(in,sampled_alpha);
    if mask_alpha*in.opacity*clip<0.5 { discard; }
    return FragmentOutput(scene.background,vec4<f32>(0.0));
}
fn transparent_output(in: Output) -> FragmentOutput {
    let clip = clip_coverage(in);
    let sampled_texture=textureSampleLevel(brush_mask,brush_sampler,in.uv,0.0);
    let mask_alpha=grain_coverage(in,sampled_texture.a);
    if mask_alpha*clip<0.001 { discard; }
    return material_output(in, in.opacity*mask_alpha*clip, sampled_texture.rgb);
}
fn premultiplied_transparent_output(in: Output) -> FragmentOutput {
    var output = transparent_output(in);
    output.color = vec4<f32>(output.color.rgb * output.color.a, output.color.a);
    return output;
}
@fragment fn fs_transparent(in: Output) -> FragmentOutput {
    return transparent_output(in);
}
@fragment fn fs_multiply(in: Output) -> FragmentOutput {
    return premultiplied_transparent_output(in);
}
@fragment fn fs_screen(in: Output) -> FragmentOutput {
    return premultiplied_transparent_output(in);
}
@fragment fn fs_add(in: Output) -> FragmentOutput {
    return premultiplied_transparent_output(in);
}
@fragment fn fs_clip_mask(in: Output) -> @location(0) vec4<f32> {
    let sampled_texture = textureSampleLevel(brush_mask, brush_sampler, in.uv, 0.0);
    let mask_alpha = grain_coverage(in, sampled_texture.a) * in.opacity * parent_clip_coverage(in);
    if mask_alpha < 0.001 { discard; }
    return vec4<f32>(1.0, 1.0, 1.0, clamp(mask_alpha, 0.0, 1.0));
}
@fragment fn fs_clip_mask_cutout(in: Output) -> @location(0) vec4<f32> {
    let sampled_alpha = textureSampleLevel(brush_mask, brush_sampler, in.uv, 0.0).a;
    let mask_alpha = grain_coverage(in, sampled_alpha) * in.opacity * parent_clip_coverage(in);
    if mask_alpha < 0.5 { discard; }
    return vec4<f32>(0.0);
}
