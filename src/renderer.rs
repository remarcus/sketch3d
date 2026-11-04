use crate::lighting::{self, Lighting, SHADOW_SIZE};
use crate::surface::GuidePreview;
use crate::texture::{self, TextureSettings};
use crate::{
    document::{BrushKind, Document, LayerBlendMode, Stroke, StrokeMaterial},
    geometry::{self, Camera, Vertex},
};
use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use glam::{Mat4, Vec3};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use wgpu::util::DeviceExt;

const MAX_LAYER_CLIP_MASK_PIXELS: u64 = 16 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniform {
    view: [f32; 16],
    light_view: [f32; 16],
    light: [f32; 4],
    settings: [f32; 4],
    shadow: [f32; 4],
    camera: [f32; 4],
    background: [f32; 4],
    resolution: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostEffectUniform {
    effects: [f32; 4],
    camera: [f32; 4],
    size: [f32; 2],
    pad: [f32; 2],
}
struct CanvasTarget {
    color: wgpu::Texture,
    view: wgpu::TextureView,
    _scene_color: wgpu::Texture,
    scene_view: wgpu::TextureView,
    _emission_color: wgpu::Texture,
    emission_view: wgpu::TextureView,
    multisample_color: Option<wgpu::TextureView>,
    multisample_emission: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    _depth_texture: wgpu::Texture,
    id: egui::TextureId,
    size: [u32; 2],
    sample_count: u32,
}

struct Mesh {
    buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
    points: usize,
    modifier_key: crate::stroke_modifiers::GeometryCacheKey,
    brush_kind: BrushKind,
    material: StrokeMaterial,
    layer_opacity: f32,
    image_tint_with_brush: bool,
    bounds: Option<(Vec3, Vec3)>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ImagePlaneVertex {
    position: [f32; 3],
    uv: [f32; 2],
    opacity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenBackgroundVertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
}

struct ImagePlaneMesh {
    plane: crate::image_planes::ImagePlane,
    buffer: wgpu::Buffer,
}

struct ImagePlaneTexture {
    // The bind group references the texture view, so retain the texture owner.
    _texture: wgpu::Texture,
    bind: wgpu::BindGroup,
}

struct ScreenBackgroundTexture {
    id: u64,
    _texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    size: [u32; 2],
    fit: BackgroundImageFit,
    alignment: [f32; 2],
    opacity: f32,
}

struct ClipMaskTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    size: [u32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundImageFit {
    Fit,
    Fill,
}

struct ObjReferenceMaterialMesh {
    mesh: Mesh,
    texture: Option<ImagePlaneTexture>,
}

#[derive(Default)]
struct ObjReferenceMeshes {
    opaque: Vec<ObjReferenceMaterialMesh>,
    translucent: Vec<ObjReferenceMaterialMesh>,
}

fn canvas_sample_count(adapter: &wgpu::Adapter) -> u32 {
    let color = adapter
        .get_texture_format_features(wgpu::TextureFormat::Rgba8UnormSrgb)
        .flags;
    let depth = adapter
        .get_texture_format_features(wgpu::TextureFormat::Depth32Float)
        .flags;
    if color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
        && depth.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
    {
        4
    } else {
        1
    }
}

fn is_translucent(stroke: &Stroke, layer_opacity: f32, layer_blend_mode: LayerBlendMode) -> bool {
    if stroke.material == StrokeMaterial::Cutout {
        return false;
    }
    layer_opacity < 0.999_999
        || layer_blend_mode != LayerBlendMode::Normal
        || stroke.brush.opacity < 0.999_999
        || stroke.brush.tip_asset_id != 0
        || stroke.fill.as_ref().and_then(|fill| fill.texture).is_some()
        || (stroke.brush.dynamics.opacity.enabled
            && stroke.brush.dynamics.opacity.minimum < 0.999_999)
        || (stroke.fill.is_none() && stroke.brush.softness > 0.0)
        || stroke.brush.texture.kind.uses_continuous_alpha()
}

fn unpremultiply_rgba(pixels: &mut [u8]) {
    let (pixels, _) = pixels.as_chunks_mut::<4>();
    for pixel in pixels {
        let alpha = f32::from(pixel[3]) / 255.0;
        if alpha <= 0.0 {
            pixel[..3].fill(0);
            continue;
        }
        for channel in &mut pixel[..3] {
            let encoded = f32::from(*channel) / 255.0;
            let straight = crate::presentation::linear_to_srgb(
                (crate::presentation::srgb_to_linear(encoded) / alpha).clamp(0.0, 1.0),
            );
            *channel = (straight * 255.0).round() as u8;
        }
    }
}

fn screen_background_vertices(
    target: [u32; 2],
    image: [u32; 2],
    fit: BackgroundImageFit,
    alignment: [f32; 2],
    opacity: f32,
) -> [ScreenBackgroundVertex; 6] {
    let target_aspect = target[0] as f32 / target[1].max(1) as f32;
    let image_aspect = image[0] as f32 / image[1].max(1) as f32;
    let (half_width, half_height, offset_x, offset_y, u0, u1, v0, v1) = match fit {
        BackgroundImageFit::Fit => {
            let (half_width, half_height, offset_x, offset_y) = if image_aspect >= target_aspect {
                let half_height = target_aspect / image_aspect;
                (
                    1.0,
                    half_height,
                    0.0,
                    (0.5 - alignment[1]) * 2.0 * (1.0 - half_height),
                )
            } else {
                let half_width = image_aspect / target_aspect;
                (
                    half_width,
                    1.0,
                    (alignment[0] - 0.5) * 2.0 * (1.0 - half_width),
                    0.0,
                )
            };
            (
                half_width,
                half_height,
                offset_x,
                offset_y,
                0.0,
                1.0,
                0.0,
                1.0,
            )
        }
        BackgroundImageFit::Fill => {
            let (u0, u1, v0, v1) = if image_aspect >= target_aspect {
                let visible = (target_aspect / image_aspect).clamp(0.0, 1.0);
                let start = (1.0 - visible) * alignment[0];
                (start, start + visible, 0.0, 1.0)
            } else {
                let visible = (image_aspect / target_aspect).clamp(0.0, 1.0);
                let start = (1.0 - visible) * alignment[1];
                (0.0, 1.0, start, start + visible)
            };
            (1.0, 1.0, 0.0, 0.0, u0, u1, v0, v1)
        }
    };
    let left = -half_width + offset_x;
    let right = half_width + offset_x;
    let top = half_height + offset_y;
    let bottom = -half_height + offset_y;
    let vertex = |position, uv| ScreenBackgroundVertex {
        position,
        uv,
        opacity,
    };
    [
        vertex([left, top], [u0, v0]),
        vertex([right, top], [u1, v0]),
        vertex([right, bottom], [u1, v1]),
        vertex([left, top], [u0, v0]),
        vertex([right, bottom], [u1, v1]),
        vertex([left, bottom], [u0, v1]),
    ]
}

fn back_to_front<'a>(
    mut strokes: Vec<(&'a Stroke, &'a Mesh)>,
    camera: &Camera,
    doc: &Document,
) -> Vec<(&'a Stroke, &'a Mesh)> {
    let axis = camera.basis().2;
    strokes.sort_by(|(stroke_a, a), (stroke_b, b)| {
        let center = |mesh: &Mesh| {
            mesh.bounds
                .map(|(min, max)| (min + max) * 0.5)
                .unwrap_or(camera.target)
        };
        (center(a) - camera.target)
            .dot(axis)
            .total_cmp(&(center(b) - camera.target).dot(axis))
            .then_with(|| {
                let layer_order = |layer_id| {
                    doc.layers
                        .iter()
                        .position(|layer| layer.id == layer_id)
                        .unwrap_or(usize::MAX)
                };
                layer_order(stroke_a.layer_id).cmp(&layer_order(stroke_b.layer_id))
            })
            .then_with(|| stroke_a.id.cmp(&stroke_b.id))
    });
    strokes
}

fn bottom_to_top<'a>(mut strokes: Vec<&'a Stroke>, doc: &Document) -> Vec<&'a Stroke> {
    let layer_order = |layer_id| {
        doc.layers
            .iter()
            .position(|layer| layer.id == layer_id)
            .unwrap_or(usize::MAX)
    };
    strokes.sort_by(|a, b| {
        layer_order(a.layer_id)
            .cmp(&layer_order(b.layer_id))
            .then(a.id.cmp(&b.id))
    });
    strokes
}

fn bottom_to_top_meshes<'a>(
    mut strokes: Vec<(&'a Stroke, &'a Mesh)>,
    doc: &Document,
) -> Vec<(&'a Stroke, &'a Mesh)> {
    let layer_order = |layer_id| {
        doc.layers
            .iter()
            .position(|layer| layer.id == layer_id)
            .unwrap_or(usize::MAX)
    };
    strokes.sort_by(|a, b| {
        layer_order(a.0.layer_id)
            .cmp(&layer_order(b.0.layer_id))
            .then(a.0.id.cmp(&b.0.id))
    });
    strokes
}

pub struct SceneRenderer {
    state: egui_wgpu::RenderState,
    triangles: wgpu::RenderPipeline,
    cutout_triangles: wgpu::RenderPipeline,
    coverage_triangles: wgpu::RenderPipeline,
    transparent_triangles: wgpu::RenderPipeline,
    blend_pipelines: HashMap<(LayerBlendMode, u32), wgpu::RenderPipeline>,
    clip_mask_pipeline: wgpu::RenderPipeline,
    clip_mask_cutout_pipeline: wgpu::RenderPipeline,
    _clip_default_texture: wgpu::Texture,
    clip_default_bind: wgpu::BindGroup,
    clip_sampler: wgpu::Sampler,
    clip_mask_cache: RefCell<HashMap<u64, Rc<ClipMaskTexture>>>,
    lines: wgpu::RenderPipeline,
    single_triangles: wgpu::RenderPipeline,
    single_cutout_triangles: wgpu::RenderPipeline,
    single_coverage_triangles: wgpu::RenderPipeline,
    single_transparent_triangles: wgpu::RenderPipeline,
    single_lines: wgpu::RenderPipeline,
    screen_background_pipeline: wgpu::RenderPipeline,
    single_screen_background_pipeline: wgpu::RenderPipeline,
    image_planes_pipeline: wgpu::RenderPipeline,
    single_image_planes_pipeline: wgpu::RenderPipeline,
    post_effects_pipeline: wgpu::RenderPipeline,
    post_effects_layout: wgpu::BindGroupLayout,
    post_effects_sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    image_plane_layout: wgpu::BindGroupLayout,
    textures: HashMap<texture::TextureCacheKey, wgpu::BindGroup>,
    meshes: HashMap<u64, Mesh>,
    image_plane_meshes: HashMap<u64, ImagePlaneMesh>,
    image_plane_textures: HashMap<u64, ImagePlaneTexture>,
    tip_textures: HashMap<u64, ImagePlaneTexture>,
    tip_atlases: HashMap<Vec<u64>, ImagePlaneTexture>,
    image_textures: HashMap<texture::ImageTextureCacheKey, ImagePlaneTexture>,
    fill_textures: HashMap<(u64, bool), ImagePlaneTexture>,
    screen_background: Option<ScreenBackgroundTexture>,
    obj_reference_meshes: HashMap<u64, ObjReferenceMeshes>,
    repeat_preview: Vec<(crate::document::Stroke, Mesh)>,
    selected: HashSet<u64>,
    grid: Option<(GuidePreview, Mesh)>,
    ground: Option<(Lighting, Mesh)>,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_view: wgpu::TextureView,
    shadow_bind: wgpu::BindGroup,
    target: Option<CanvasTarget>,
    pub lighting: Lighting,
    pub background: [f32; 3],
    pub grain: f32,
    pub toon_shading: bool,
    pub post_effects: crate::presentation::PostEffects,
    pub adapter: String,
}
impl SceneRenderer {
    pub fn new(state: egui_wgpu::RenderState) -> Self {
        let device = &state.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Stroke shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("stroke.wgsl").into()),
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Scene uniforms"),
            contents: bytemuck::bytes_of(&SceneUniform {
                view: Mat4::IDENTITY.to_cols_array(),
                light_view: Mat4::IDENTITY.to_cols_array(),
                light: [0.0; 4],
                settings: [0.0; 4],
                shadow: [0.0; 4],
                camera: [0.0; 4],
                background: [0.0; 4],
                resolution: [1.0, 1.0, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Brush mask layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let clip_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Layer clip mask sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let clip_default_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Unclipped layer mask"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        state.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &clip_default_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let clip_default_view = clip_default_texture.create_view(&Default::default());
        let clip_default_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Unclipped layer mask binding"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&clip_default_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&clip_sampler),
                },
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shadow sampling"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Directional shadow map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Shadow comparison sampler"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shadow map"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shadow shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shadow pass layout"),
            bind_group_layouts: &[&layout, &texture_layout],
            push_constant_ranges: &[],
        });
        let shadow_pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("Masked shadow casters"),layout:Some(&depth_layout),
            vertex:wgpu::VertexState {module:&shadow_shader,entry_point:Some("vs_shadow"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout {array_stride:std::mem::size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2,4=>Float32,5=>Float32]}]},
            fragment:Some(wgpu::FragmentState {module:&shadow_shader,entry_point:Some("fs_shadow"),compilation_options:Default::default(),targets:&[]}),
            primitive:wgpu::PrimitiveState {cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:true,depth_compare:wgpu::CompareFunction::LessEqual,stencil:Default::default(),bias:wgpu::DepthBiasState {constant:1,slope_scale:1.0,clamp:0.0}}),
            multisample:Default::default(),multiview:None,cache:None,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                &layout,
                &texture_layout,
                &shadow_layout,
                &texture_layout,
                &texture_layout,
            ],
            push_constant_ranges: &[],
        });
        let sample_count = canvas_sample_count(&state.adapter);
        let create = |topology,
                      transparent: bool,
                      alpha_blend: bool,
                      cutout: bool,
                      pipeline_samples,
                      blend_mode: Option<LayerBlendMode>| {
            let entry_point = if cutout {
                "fs_cutout"
            } else {
                match blend_mode {
                    Some(LayerBlendMode::Multiply) => "fs_multiply",
                    Some(LayerBlendMode::Screen) => "fs_screen",
                    Some(LayerBlendMode::Add) => "fs_add",
                    _ if transparent => "fs_transparent",
                    _ => "fs_main",
                }
            };
            let color_blend = if cutout {
                Some(wgpu::BlendState::REPLACE)
            } else {
                match blend_mode {
                    Some(LayerBlendMode::Multiply) => Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Dst,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    Some(LayerBlendMode::Screen) => Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrc,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    Some(LayerBlendMode::Add) => Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    _ if alpha_blend => Some(wgpu::BlendState::ALPHA_BLENDING),
                    _ => None,
                }
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("3D strokes"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs_main"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout {array_stride:std::mem::size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2,4=>Float32,5=>Float32,6=>Float32x2,7=>Float32x2,8=>Float32x3]}]},
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some(entry_point),compilation_options:Default::default(),targets:&[
                Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba8UnormSrgb,blend:color_blend,write_mask:wgpu::ColorWrites::ALL}),
                Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba8Unorm,blend:Some(wgpu::BlendState {color:wgpu::BlendComponent {src_factor:if cutout {wgpu::BlendFactor::Zero} else {wgpu::BlendFactor::One},dst_factor:if cutout {wgpu::BlendFactor::Zero} else {wgpu::BlendFactor::One},operation:wgpu::BlendOperation::Add},alpha:wgpu::BlendComponent {src_factor:if cutout {wgpu::BlendFactor::Zero} else {wgpu::BlendFactor::One},dst_factor:if cutout {wgpu::BlendFactor::Zero} else {wgpu::BlendFactor::One},operation:wgpu::BlendOperation::Add}}),write_mask:wgpu::ColorWrites::ALL})
            ]}),
            primitive:wgpu::PrimitiveState {topology,cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:(!transparent || cutout) && topology != wgpu::PrimitiveTopology::LineList,depth_compare:wgpu::CompareFunction::LessEqual,stencil:Default::default(),bias:Default::default()}),
            multisample:wgpu::MultisampleState {count:pipeline_samples,mask:!0,alpha_to_coverage_enabled:pipeline_samples > 1 && !transparent && !alpha_blend && !cutout},multiview:None,cache:None,
        })
        };
        let clip_mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Layer clip coverage mask"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2,4=>Float32,5=>Float32,6=>Float32x2,7=>Float32x2,8=>Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_clip_mask"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::RED,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let clip_mask_cutout_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Layer clip cutout mask"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2,4=>Float32,5=>Float32,6=>Float32x2,7=>Float32x2,8=>Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_clip_mask_cutout"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            false,
            false,
            false,
            sample_count,
            None,
        );
        let cutout_triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            false,
            false,
            true,
            sample_count,
            None,
        );
        let coverage_triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            false,
            true,
            false,
            sample_count,
            None,
        );
        let transparent_triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            true,
            true,
            false,
            sample_count,
            None,
        );
        let lines = create(
            wgpu::PrimitiveTopology::LineList,
            false,
            false,
            false,
            sample_count,
            None,
        );
        let single_triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            false,
            false,
            false,
            1,
            None,
        );
        let single_cutout_triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            false,
            false,
            true,
            1,
            None,
        );
        let single_coverage_triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            false,
            true,
            false,
            1,
            None,
        );
        let single_transparent_triangles = create(
            wgpu::PrimitiveTopology::TriangleList,
            true,
            true,
            false,
            1,
            None,
        );
        let single_lines = create(
            wgpu::PrimitiveTopology::LineList,
            false,
            false,
            false,
            1,
            None,
        );
        let mut blend_pipelines = HashMap::new();
        for mode in [
            LayerBlendMode::Multiply,
            LayerBlendMode::Screen,
            LayerBlendMode::Add,
        ] {
            for samples in if sample_count == 1 {
                vec![1]
            } else {
                vec![sample_count, 1]
            } {
                blend_pipelines.insert(
                    (mode, samples),
                    create(
                        wgpu::PrimitiveTopology::TriangleList,
                        true,
                        true,
                        false,
                        samples,
                        Some(mode),
                    ),
                );
            }
        }
        let image_plane_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Image plane texture layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Image plane shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("image_plane.wgsl").into()),
        });
        let screen_background_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Screen background image shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("screen_background.wgsl").into()),
        });
        let screen_background_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Screen background image layout"),
                bind_group_layouts: &[&image_plane_layout],
                push_constant_ranges: &[],
            });
        let create_screen_background_pipeline = |pipeline_samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Screen-aligned background image"),
                layout: Some(&screen_background_layout),
                vertex: wgpu::VertexState {
                    module: &screen_background_shader,
                    entry_point: Some("vs_background"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ScreenBackgroundVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &screen_background_shader,
                    entry_point: Some("fs_background"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }), Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::empty(),
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: pipeline_samples,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
                cache: None,
            })
        };
        let screen_background_pipeline = create_screen_background_pipeline(sample_count);
        let single_screen_background_pipeline = create_screen_background_pipeline(1);
        let image_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Image plane pipeline layout"),
                bind_group_layouts: &[&layout, &image_plane_layout],
                push_constant_ranges: &[],
            });
        let create_image_planes_pipeline = |pipeline_samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Textured image planes"),
            layout: Some(&image_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &image_shader,
                entry_point: Some("vs_image"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ImagePlaneVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
                entry_point: Some("fs_image"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::empty(),
                    }),
                ],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: pipeline_samples, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        })
        };
        let image_planes_pipeline = create_image_planes_pipeline(sample_count);
        let single_image_planes_pipeline = create_image_planes_pipeline(1);
        let effects_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Post-processing effects layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let effects_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Canvas post-processing shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_effects.wgsl").into()),
        });
        let effects_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Post-processing effects pipeline layout"),
                bind_group_layouts: &[&effects_layout],
                push_constant_ranges: &[],
            });
        let post_effects_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Canvas post-processing effects"),
                layout: Some(&effects_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &effects_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &effects_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });
        let post_effects_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Post-processing sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let info = state.adapter.get_info();
        let adapter = format!("{} / {:?}", info.name, info.backend);
        Self {
            state,
            triangles,
            cutout_triangles,
            coverage_triangles,
            transparent_triangles,
            blend_pipelines,
            clip_mask_pipeline,
            clip_mask_cutout_pipeline,
            _clip_default_texture: clip_default_texture,
            clip_default_bind,
            clip_sampler,
            clip_mask_cache: RefCell::new(HashMap::new()),
            lines,
            single_triangles,
            single_cutout_triangles,
            single_coverage_triangles,
            single_transparent_triangles,
            single_lines,
            screen_background_pipeline,
            single_screen_background_pipeline,
            image_planes_pipeline,
            single_image_planes_pipeline,
            post_effects_pipeline,
            post_effects_layout: effects_layout,
            post_effects_sampler,
            uniform,
            bind,
            texture_layout,
            image_plane_layout,
            textures: HashMap::new(),
            meshes: HashMap::new(),
            image_plane_meshes: HashMap::new(),
            image_plane_textures: HashMap::new(),
            tip_textures: HashMap::new(),
            tip_atlases: HashMap::new(),
            image_textures: HashMap::new(),
            fill_textures: HashMap::new(),
            screen_background: None,
            obj_reference_meshes: HashMap::new(),
            repeat_preview: Vec::new(),
            selected: HashSet::new(),
            grid: None,
            ground: None,
            lighting: Lighting::default(),
            shadow_pipeline,
            shadow_view,
            shadow_bind,
            target: None,
            background: crate::presentation::Preferences::default().background,
            grain: 0.0,
            toon_shading: false,
            post_effects: crate::presentation::PostEffects::default(),
            adapter,
        }
    }
    fn create_mask(&self, settings: TextureSettings) -> wgpu::BindGroup {
        let device = &self.state.device;
        let mask = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Procedural brush mask"),
            size: wgpu::Extent3d {
                width: texture::SIZE,
                height: texture::SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.state.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &mask,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texture::rgba(settings),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(texture::SIZE * 4),
                rows_per_image: Some(texture::SIZE),
            },
            wgpu::Extent3d {
                width: texture::SIZE,
                height: texture::SIZE,
                depth_or_array_layers: 1,
            },
        );
        let view = mask.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Repeat brush mask"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Brush mask"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        })
    }

    fn create_tip_texture(
        &self,
        asset: &crate::brush_tip::BrushTipAsset,
    ) -> Result<ImagePlaneTexture, String> {
        self.create_portable_image_texture(asset, false, "Portable brush image tip")
    }

    fn create_portable_image_texture(
        &self,
        asset: &crate::brush_tip::BrushTipAsset,
        repeat: bool,
        label: &'static str,
    ) -> Result<ImagePlaneTexture, String> {
        let decoded = asset.decoded()?;
        self.create_image_texture(&decoded, repeat, label)
    }

    fn create_image_texture(
        &self,
        image: &crate::image_planes::ImageAsset,
        repeat: bool,
        label: &'static str,
    ) -> Result<ImagePlaneTexture, String> {
        let wrap = if repeat {
            crate::obj_reference::ObjTextureWrap::Repeat
        } else {
            crate::obj_reference::ObjTextureWrap::Clamp
        };
        self.create_image_texture_with_sampler(
            image,
            crate::obj_reference::ObjTextureSampler {
                wrap_s: wrap,
                wrap_t: wrap,
                mag_filter: crate::obj_reference::ObjTextureFilter::Linear,
                min_filter: crate::obj_reference::ObjTextureFilter::Linear,
            },
            label,
        )
    }

    fn create_image_texture_with_sampler(
        &self,
        image: &crate::image_planes::ImageAsset,
        sampler_settings: crate::obj_reference::ObjTextureSampler,
        label: &'static str,
    ) -> Result<ImagePlaneTexture, String> {
        let (width, height) = (image.size[0], image.size[1]);
        let expected_bytes = u64::from(width) * u64::from(height) * 4;
        if width == 0 || height == 0 || expected_bytes != image.pixels.len() as u64 {
            return Err("Portable image texture dimensions do not match its pixels".into());
        }
        let texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.state.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        let wrap = |mode| match mode {
            crate::obj_reference::ObjTextureWrap::Repeat => wgpu::AddressMode::Repeat,
            crate::obj_reference::ObjTextureWrap::Clamp => wgpu::AddressMode::ClampToEdge,
            crate::obj_reference::ObjTextureWrap::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
        };
        let filter = |mode| match mode {
            crate::obj_reference::ObjTextureFilter::Nearest
            | crate::obj_reference::ObjTextureFilter::NearestMipmapNearest
            | crate::obj_reference::ObjTextureFilter::NearestMipmapLinear => {
                wgpu::FilterMode::Nearest
            }
            crate::obj_reference::ObjTextureFilter::Linear
            | crate::obj_reference::ObjTextureFilter::LinearMipmapNearest
            | crate::obj_reference::ObjTextureFilter::LinearMipmapLinear => {
                wgpu::FilterMode::Linear
            }
        };
        let mipmap_filter = |mode| match mode {
            crate::obj_reference::ObjTextureFilter::NearestMipmapLinear
            | crate::obj_reference::ObjTextureFilter::LinearMipmapLinear => {
                wgpu::FilterMode::Linear
            }
            _ => wgpu::FilterMode::Nearest,
        };
        let sampler = self.state.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: wrap(sampler_settings.wrap_s),
            address_mode_v: wrap(sampler_settings.wrap_t),
            mag_filter: filter(sampler_settings.mag_filter),
            min_filter: filter(sampler_settings.min_filter),
            mipmap_filter: mipmap_filter(sampler_settings.min_filter),
            ..Default::default()
        });
        let bind = self
            .state
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
        Ok(ImagePlaneTexture {
            _texture: texture,
            bind,
        })
    }

    fn prepare_tip_textures(&mut self, doc: &Document) {
        let mut needed = HashSet::new();
        let mut atlas_needed = HashSet::new();
        for stroke in doc
            .strokes
            .iter()
            .filter(|stroke| doc.visible(stroke))
            .chain(self.repeat_preview.iter().map(|(stroke, _)| stroke))
        {
            let ids = stroke.brush.tip_variant_ids();
            if ids.len() > 1 {
                atlas_needed.insert(ids.clone());
            } else {
                needed.extend(ids);
            }
        }
        self.tip_textures.retain(|id, _| needed.contains(id));
        for id in needed {
            if self.tip_textures.contains_key(&id) {
                continue;
            }
            let Some(asset) = doc.brush_tip_assets.iter().find(|asset| asset.id == id) else {
                continue;
            };
            if let Ok(texture) = self.create_tip_texture(asset) {
                self.tip_textures.insert(id, texture);
            }
        }
        self.tip_atlases.retain(|ids, _| atlas_needed.contains(ids));
        for ids in atlas_needed {
            if self.tip_atlases.contains_key(&ids) {
                continue;
            }
            let assets: Option<Vec<_>> = ids
                .iter()
                .map(|id| doc.brush_tip_assets.iter().find(|asset| asset.id == *id))
                .collect();
            let Some(assets) = assets else {
                continue;
            };
            if let Ok(atlas) = crate::brush_tip::build_tip_atlas(&assets) {
                if let Ok(texture) =
                    self.create_image_texture(&atlas, false, "Multiple brush-tip variants")
                {
                    self.tip_atlases.insert(ids, texture);
                }
            }
        }
        let mut image_needed: HashMap<
            texture::ImageTextureCacheKey,
            texture::ImageTextureSettings,
        > = doc
            .strokes
            .iter()
            .filter(|stroke| doc.visible(stroke))
            .filter_map(|stroke| stroke.brush.texture.image)
            .chain(
                self.repeat_preview
                    .iter()
                    .filter_map(|(stroke, _)| stroke.brush.texture.image),
            )
            .map(|settings| (settings.cache_key(), settings))
            .collect();
        self.image_textures
            .retain(|key, _| image_needed.contains_key(key));
        for (key, settings) in image_needed.drain() {
            if self.image_textures.contains_key(&key) {
                continue;
            }
            let Some(asset) = doc
                .brush_tip_assets
                .iter()
                .find(|asset| asset.id == settings.asset_id)
            else {
                continue;
            };
            if let Ok(pixels) = asset.repeating_texture_pixels(settings) {
                if let Ok(texture) =
                    self.create_image_texture(&pixels, true, "Portable repeating brush texture")
                {
                    self.image_textures.insert(key, texture);
                }
            }
        }
        let mut fill_needed: HashSet<(u64, bool)> = doc
            .strokes
            .iter()
            .filter(|stroke| doc.visible(stroke))
            .filter_map(|stroke| stroke.fill.as_ref().and_then(|fill| fill.texture))
            .map(|texture| (texture.asset_id, texture.repeat))
            .collect();
        fill_needed.extend(
            self.repeat_preview
                .iter()
                .filter_map(|(stroke, _)| stroke.fill.as_ref().and_then(|fill| fill.texture))
                .map(|texture| (texture.asset_id, texture.repeat)),
        );
        self.fill_textures
            .retain(|key, _| fill_needed.contains(key));
        for (id, repeat) in fill_needed {
            if self.fill_textures.contains_key(&(id, repeat)) {
                continue;
            }
            let Some(asset) = doc.brush_tip_assets.iter().find(|asset| asset.id == id) else {
                continue;
            };
            if let Ok(texture) =
                self.create_portable_image_texture(asset, repeat, "Portable planar fill texture")
            {
                self.fill_textures.insert((id, repeat), texture);
            }
        }
    }

    fn brush_texture(&self, stroke: &Stroke) -> &wgpu::BindGroup {
        let tip_texture = if stroke.brush.uses_tip_atlas() {
            self.tip_atlases.get(&stroke.brush.tip_variant_ids())
        } else {
            self.tip_textures.get(&stroke.brush.tip_asset_id)
        };
        stroke
            .fill
            .as_ref()
            .and_then(|fill| fill.texture)
            .and_then(|texture| self.fill_textures.get(&(texture.asset_id, texture.repeat)))
            .or(tip_texture)
            .or_else(|| {
                stroke
                    .brush
                    .texture
                    .image
                    .and_then(|settings| self.image_textures.get(&settings.cache_key()))
            })
            .map(|texture| &texture.bind)
            .unwrap_or(&self.textures[&texture::key(stroke.brush.texture)])
    }

    fn grain_texture(&self, stroke: &Stroke) -> &wgpu::BindGroup {
        stroke
            .brush
            .texture
            .image
            .and_then(|settings| self.image_textures.get(&settings.cache_key()))
            .map(|texture| &texture.bind)
            .unwrap_or(&self.textures[&texture::key(stroke.brush.texture)])
    }
    pub fn invalidate(&mut self) {
        self.meshes.clear();
    }

    /// Configure a screen-aligned image behind artwork. Source pixels are RGBA8
    /// in top-to-bottom row order. This asset is independent of camera rays and
    /// is only drawn in opaque-background canvas/image renders.
    pub fn set_screen_background_image(
        &mut self,
        id: u64,
        rgba: &[u8],
        size: [u32; 2],
        fit: BackgroundImageFit,
        alignment: [f32; 2],
        opacity: f32,
    ) -> Result<(), String> {
        let [width, height] = size;
        let byte_len = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("Background image dimensions overflow")?;
        if id == 0
            || width == 0
            || height == 0
            || width > self.state.device.limits().max_texture_dimension_2d
            || height > self.state.device.limits().max_texture_dimension_2d
            || byte_len > 12_000_000 * 4
            || byte_len != rgba.len() as u64
            || alignment
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            || !opacity.is_finite()
            || !(0.0..=1.0).contains(&opacity)
        {
            return Err("Background image has invalid dimensions, alignment, or opacity".into());
        }

        if self
            .screen_background
            .as_ref()
            .is_none_or(|background| background.id != id || background.size != size)
        {
            let texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Presentation background image"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.state.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&Default::default());
            let sampler = self.state.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Presentation background image sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let bind = self
                .state
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Presentation background image"),
                    layout: &self.image_plane_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
            self.screen_background = Some(ScreenBackgroundTexture {
                id,
                _texture: texture,
                bind,
                size,
                fit,
                alignment,
                opacity,
            });
        } else if let Some(background) = &mut self.screen_background {
            background.fit = fit;
            background.alignment = alignment;
            background.opacity = opacity;
        }
        Ok(())
    }

    pub fn clear_screen_background_image(&mut self) {
        self.screen_background = None;
    }

    /// Upload decoded RGBA pixels for a document image plane. The asset is kept
    /// by plane ID and can be replaced after an edit or document reload.
    pub fn set_image_plane_asset(
        &mut self,
        id: u64,
        asset: &crate::image_planes::ImageAsset,
    ) -> Result<(), String> {
        let [width, height] = asset.size;
        let byte_len = (width as u64)
            .checked_mul(height as u64)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("Image-plane dimensions overflow")?;
        if id == 0
            || width == 0
            || height == 0
            || width > self.state.device.limits().max_texture_dimension_2d
            || height > self.state.device.limits().max_texture_dimension_2d
            || byte_len != asset.pixels.len() as u64
            || byte_len > crate::image_planes::MAX_IMAGE_PIXELS * 4
        {
            return Err("Image-plane asset has invalid dimensions or RGBA data".into());
        }
        let texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Image plane asset"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.state.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &asset.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        let sampler = self.state.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Image plane sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind = self
            .state
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Image plane asset bind group"),
                layout: &self.image_plane_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
        self.image_plane_textures.insert(
            id,
            ImagePlaneTexture {
                _texture: texture,
                bind,
            },
        );
        Ok(())
    }

    pub fn remove_image_plane_asset(&mut self, id: u64) {
        self.image_plane_textures.remove(&id);
        self.image_plane_meshes.remove(&id);
    }

    pub fn has_image_plane_asset(&self, id: u64) -> bool {
        self.image_plane_textures.contains_key(&id)
    }

    pub fn clear_image_plane_assets(&mut self) {
        self.image_plane_textures.clear();
        self.image_plane_meshes.clear();
    }

    pub fn set_obj_reference_model(
        &mut self,
        id: u64,
        model: &crate::obj_reference::ObjModel,
        center: Vec3,
        rotation: glam::Quat,
        scale: f32,
    ) -> Result<(), String> {
        if id == 0
            || !center.is_finite()
            || !rotation.is_finite()
            || (rotation.length_squared() - 1.0).abs() > 0.001
            || !scale.is_finite()
            || !(0.001..=10_000.0).contains(&scale)
        {
            return Err("OBJ reference transform is invalid".into());
        }
        let mut opaque: HashMap<usize, Vec<Vertex>> = HashMap::new();
        let mut translucent: HashMap<usize, Vec<Vertex>> = HashMap::new();
        for triangle in &model.triangles {
            let material = model.material(triangle);
            let material_index = triangle.material.unwrap_or(0) as usize;
            let color = material.map_or([1.0; 3], |material| material.diffuse);
            let opacity = material.map_or(1.0, |material| material.opacity);
            let alpha_mode = material.map_or(
                crate::obj_reference::ObjMaterialAlphaMode::Opaque,
                |material| material.alpha_mode,
            );
            let is_translucent = alpha_mode == crate::obj_reference::ObjMaterialAlphaMode::Blend;
            let target = if is_translucent {
                translucent.entry(material_index).or_default()
            } else {
                opaque.entry(material_index).or_default()
            };
            let alpha_mode_marker = match alpha_mode {
                crate::obj_reference::ObjMaterialAlphaMode::Opaque => 3.0,
                crate::obj_reference::ObjMaterialAlphaMode::Mask => 2.0,
                crate::obj_reference::ObjMaterialAlphaMode::Blend => 1.0,
            };
            let alpha_cutoff = material.map_or(0.5, |material| material.alpha_cutoff);
            for source in model.render_vertices(triangle)? {
                let position = center + rotation * (source.position * scale);
                if !position.is_finite() || position.abs().max_element() > 10_000.0 {
                    return Err("Transformed OBJ geometry exceeds document bounds".into());
                }
                target.push(Vertex {
                    material: 0.0,
                    emission: material.map_or([0.0; 3], |material| material.emissive_factor),
                    grain_uv: [0.0; 2],
                    grain_dynamics: [alpha_cutoff, alpha_mode_marker],
                    gradient_t: 0.0,
                    position: position.to_array(),
                    normal: (rotation * source.normal).normalize_or_zero().to_array(),
                    uv: source.texcoord.map_or([0.0; 2], |uv| uv.to_array()),
                    color,
                    opacity,
                });
            }
        }
        let build_material_meshes = |this: &Self,
                                     vertices: HashMap<usize, Vec<Vertex>>|
         -> Result<Vec<ObjReferenceMaterialMesh>, String> {
            let mut batches = vertices.into_iter().collect::<Vec<_>>();
            batches.sort_by_key(|(index, _)| *index);
            batches
                .into_iter()
                .map(|(material_index, vertices)| {
                    let material = model.materials.get(material_index);
                    let texture = material
                        .and_then(|material| material.base_color_texture.as_ref())
                        .map(|image| {
                            this.create_image_texture_with_sampler(
                                image,
                                material
                                    .and_then(|material| material.texture_sampler)
                                    .unwrap_or_default(),
                                "GLB base-color texture",
                            )
                        })
                        .transpose()?;
                    Ok(ObjReferenceMaterialMesh {
                        mesh: this.mesh(&vertices, 0, BrushKind::Round, StrokeMaterial::Shaded),
                        texture,
                    })
                })
                .collect()
        };
        self.obj_reference_meshes.insert(
            id,
            ObjReferenceMeshes {
                opaque: build_material_meshes(self, opaque)?,
                translucent: build_material_meshes(self, translucent)?,
            },
        );
        Ok(())
    }

    pub fn clear_obj_reference_models(&mut self) {
        self.obj_reference_meshes.clear();
    }

    pub fn has_obj_reference_model(&self, id: u64) -> bool {
        self.obj_reference_meshes.contains_key(&id)
    }

    fn image_plane_mesh(&self, plane: &crate::image_planes::ImagePlane) -> ImagePlaneMesh {
        let half = plane.size * 0.5;
        let corners = [
            (glam::Vec3::new(-half.x, half.y, 0.0), [0.0, 0.0]),
            (glam::Vec3::new(-half.x, -half.y, 0.0), [0.0, 1.0]),
            (glam::Vec3::new(half.x, -half.y, 0.0), [1.0, 1.0]),
            (glam::Vec3::new(-half.x, half.y, 0.0), [0.0, 0.0]),
            (glam::Vec3::new(half.x, -half.y, 0.0), [1.0, 1.0]),
            (glam::Vec3::new(half.x, half.y, 0.0), [1.0, 0.0]),
        ];
        let vertices: Vec<_> = corners
            .into_iter()
            .map(|(local, uv)| ImagePlaneVertex {
                position: (plane.origin + plane.rotation * local).to_array(),
                uv,
                opacity: plane.opacity,
            })
            .collect();
        let buffer = self
            .state
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Image plane quad"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        ImagePlaneMesh {
            plane: plane.clone(),
            buffer,
        }
    }
    pub fn invalidate_ids(&mut self, ids: &HashSet<u64>) {
        for id in ids {
            if let Some(mesh) = self.meshes.get_mut(id) {
                mesh.points = usize::MAX;
            }
        }
    }
    pub fn set_selection(&mut self, selected: &HashSet<u64>) {
        let changed: HashSet<_> = self
            .selected
            .symmetric_difference(selected)
            .copied()
            .collect();
        self.invalidate_ids(&changed);
        self.selected.clone_from(selected);
    }
    pub fn sample_count(&self) -> u32 {
        self.target.as_ref().map_or(1, |target| target.sample_count)
    }
    pub fn set_repeat_preview(&mut self, strokes: &[crate::document::Stroke], doc: &Document) {
        if self
            .repeat_preview
            .iter()
            .map(|(stroke, _)| stroke)
            .eq(strokes.iter())
            && self.repeat_preview.iter().all(|(stroke, mesh)| {
                (mesh.layer_opacity - doc.layer_opacity(stroke.layer_id)).abs() < 0.000_001
            })
        {
            return;
        }
        self.repeat_preview = strokes
            .iter()
            .map(|stroke| {
                let mut vertices = geometry::stroke_mesh(stroke);
                for vertex in &mut vertices {
                    vertex.color = [0.12, 0.78, 0.92];
                }
                (
                    stroke.clone(),
                    self.layer_mesh(
                        &vertices,
                        stroke.points.len(),
                        stroke.brush.kind,
                        stroke.material,
                        doc.layer_opacity(stroke.layer_id),
                    ),
                )
            })
            .collect();
    }
    fn mesh(
        &self,
        vertices: &[Vertex],
        points: usize,
        brush_kind: BrushKind,
        material: StrokeMaterial,
    ) -> Mesh {
        let data = bytemuck::cast_slice(vertices);
        let capacity = data.len().max(4).next_power_of_two();
        let buffer = self.state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Stroke vertices"),
            size: capacity as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !data.is_empty() {
            self.state.queue.write_buffer(&buffer, 0, data);
        }
        Mesh {
            buffer,
            capacity,
            count: vertices.len() as u32,
            points,
            modifier_key: crate::stroke_modifiers::GeometryCacheKey::identity(points),
            brush_kind,
            material,
            layer_opacity: 1.0,
            image_tint_with_brush: true,
            bounds: vertex_bounds(vertices),
        }
    }
    fn layer_mesh(
        &self,
        vertices: &[Vertex],
        points: usize,
        brush_kind: BrushKind,
        material: StrokeMaterial,
        layer_opacity: f32,
    ) -> Mesh {
        let mut vertices = vertices.to_vec();
        for vertex in &mut vertices {
            vertex.opacity *= layer_opacity;
        }
        let mut mesh = self.mesh(&vertices, points, brush_kind, material);
        mesh.layer_opacity = layer_opacity;
        mesh
    }
    #[allow(clippy::too_many_arguments)]
    fn record_post_effects(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        emission: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        size: [u32; 2],
        camera: &Camera,
        timestamp_queries: Option<&wgpu::QuerySet>,
    ) {
        let effects = self.post_effects;
        let orthographic = camera.projection == crate::geometry::Projection::Orthographic;
        let uniform = PostEffectUniform {
            effects: [
                effects.glow.clamp(0.0, 1.0),
                effects.focus_distance.clamp(0.0, 1000.0),
                effects.depth_of_field.clamp(0.0, 16.0),
                effects.pixel_size.clamp(1.0, 16.0),
            ],
            camera: [
                camera.distance,
                f32::from(orthographic),
                if orthographic {
                    camera.distance - 500.0
                } else {
                    0.01
                },
                if orthographic {
                    camera.distance + 500.0
                } else {
                    1000.0
                },
            ],
            size: [size[0] as f32, size[1] as f32],
            pad: [0.0; 2],
        };
        let buffer = self
            .state
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Post-processing parameters"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = self
            .state
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Post-processing inputs"),
                layout: &self.post_effects_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.post_effects_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(emission),
                    },
                ],
            });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Canvas post-processing"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamp_queries.map(|query_set| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(1),
            }),
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.post_effects_pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    fn record_layer_clip_masks(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        doc: &Document,
        size: [u32; 2],
        meshes: &HashMap<u64, &Mesh>,
    ) -> HashMap<u64, Rc<ClipMaskTexture>> {
        let mut masks = HashMap::new();
        let needed: HashSet<_> = doc
            .layers
            .windows(2)
            .filter(|layers| layers[1].clip_to_below)
            .map(|layers| layers[0].id)
            .collect();
        let mask_count = doc
            .layers
            .windows(2)
            .filter(|layers| layers[1].clip_to_below)
            .count()
            .max(1) as u64;
        let full_pixels = u64::from(size[0]) * u64::from(size[1]);
        let pixels_per_mask = (MAX_LAYER_CLIP_MASK_PIXELS / mask_count).max(1);
        let scale = (pixels_per_mask as f64 / full_pixels.max(1) as f64)
            .sqrt()
            .min(1.0);
        let mask_size = [
            (f64::from(size[0]) * scale).floor().max(1.0) as u32,
            (f64::from(size[1]) * scale).floor().max(1.0) as u32,
        ];
        let mut cache = self.clip_mask_cache.borrow_mut();
        cache.retain(|id, mask| needed.contains(id) && mask.size == mask_size);
        for (index, layer) in doc.layers.iter().enumerate() {
            if !doc
                .layers
                .get(index + 1)
                .is_some_and(|above| above.clip_to_below)
            {
                continue;
            }
            if !cache.contains_key(&layer.id) {
                let texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Layer coverage mask"),
                    size: wgpu::Extent3d {
                        width: mask_size[0],
                        height: mask_size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let bind = self
                    .state
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Layer coverage mask binding"),
                        layout: &self.texture_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.clip_sampler),
                            },
                        ],
                    });
                cache.insert(
                    layer.id,
                    Rc::new(ClipMaskTexture {
                        _texture: texture,
                        view,
                        bind,
                        size: mask_size,
                    }),
                );
            }
            let mask = Rc::clone(&cache[&layer.id]);
            let parent_mask = if layer.clip_to_below && index > 0 {
                masks
                    .get(&doc.layers[index - 1].id)
                    .map_or(&self.clip_default_bind, |mask: &Rc<ClipMaskTexture>| {
                        &mask.bind
                    })
            } else {
                &self.clip_default_bind
            };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Layer coverage mask pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &mask.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if layer.visible {
                    pass.set_bind_group(0, &self.bind, &[]);
                    pass.set_bind_group(4, parent_mask, &[]);
                    let mut layer_strokes: Vec<_> = doc
                        .strokes
                        .iter()
                        .filter(|stroke| stroke.layer_id == layer.id && doc.visible(stroke))
                        .collect();
                    layer_strokes.sort_by_key(|stroke| stroke.id);
                    for stroke in layer_strokes {
                        let Some(mesh) = meshes.get(&stroke.id) else {
                            continue;
                        };
                        pass.set_pipeline(if stroke.material == StrokeMaterial::Cutout {
                            &self.clip_mask_cutout_pipeline
                        } else {
                            &self.clip_mask_pipeline
                        });
                        pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                        pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                        pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                        pass.draw(0..mesh.count, 0..1);
                    }
                }
            }
            masks.insert(layer.id, mask);
        }
        masks
    }

    fn clip_binding<'a>(
        &'a self,
        doc: &Document,
        layer_id: u64,
        masks: &'a HashMap<u64, Rc<ClipMaskTexture>>,
    ) -> &'a wgpu::BindGroup {
        doc.layer_clip_source(layer_id)
            .and_then(|source| masks.get(&source))
            .map_or(&self.clip_default_bind, |mask| &mask.bind)
    }

    pub fn render(
        &mut self,
        doc: &Document,
        camera: &Camera,
        plane: GuidePreview,
        size: [u32; 2],
    ) -> egui::TextureId {
        self.prepare_tip_textures(doc);
        let limit = self.state.device.limits().max_texture_dimension_2d;
        let size = [size[0].clamp(1, limit), size[1].clamp(1, limit)];
        let effects = self.post_effects;
        let effects_enabled =
            effects.glow > 0.0 || effects.depth_of_field > 0.0 || effects.pixel_size > 1.0;
        let requested_samples = if effects_enabled {
            1
        } else {
            canvas_sample_count(&self.state.adapter)
        };
        let (triangles, cutout_triangles, transparent_triangles, lines, image_planes_pipeline) =
            if effects_enabled {
                (
                    &self.single_triangles,
                    &self.single_cutout_triangles,
                    &self.single_transparent_triangles,
                    &self.single_lines,
                    &self.single_image_planes_pipeline,
                )
            } else {
                (
                    &self.triangles,
                    &self.cutout_triangles,
                    &self.transparent_triangles,
                    &self.lines,
                    &self.image_planes_pipeline,
                )
            };
        let screen_background_pipeline = if effects_enabled {
            &self.single_screen_background_pipeline
        } else {
            &self.screen_background_pipeline
        };
        if self
            .target
            .as_ref()
            .is_none_or(|t| t.size != size || t.sample_count != requested_samples)
        {
            if let Some(target) = self.target.take() {
                self.state.renderer.write().free_texture(&target.id);
            }
            let extent = wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            };
            let sample_count = requested_samples;
            let color = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D canvas post-processing output"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let scene_color = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D canvas scene color"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let scene_view = scene_color.create_view(&Default::default());
            let emission_color = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas emissive contribution"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let emission_view = emission_color.create_view(&Default::default());
            let multisample_color = (sample_count > 1).then(|| {
                self.state
                    .device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("3D canvas multisample color"),
                        size: extent,
                        mip_level_count: 1,
                        sample_count,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            });
            let multisample_emission = (sample_count > 1).then(|| {
                self.state
                    .device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Canvas multisample emissive contribution"),
                        size: extent,
                        mip_level_count: 1,
                        sample_count,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            });
            let depth = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas depth"),
                size: extent,
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | if sample_count == 1 {
                        wgpu::TextureUsages::TEXTURE_BINDING
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            });
            let view = color.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let id = self.state.renderer.write().register_native_texture(
                &self.state.device,
                &view,
                wgpu::FilterMode::Linear,
            );
            self.target = Some(CanvasTarget {
                color,
                view,
                _scene_color: scene_color,
                scene_view,
                _emission_color: emission_color,
                emission_view,
                multisample_color,
                multisample_emission,
                depth: depth_view,
                _depth_texture: depth,
                id,
                size,
                sample_count,
            });
        }
        let solid = TextureSettings::default();
        let needed: HashSet<_> = doc
            .strokes
            .iter()
            .map(|s| texture::key(s.brush.texture))
            .chain(
                self.repeat_preview
                    .iter()
                    .map(|(stroke, _)| texture::key(stroke.brush.texture)),
            )
            .chain(std::iter::once(texture::key(solid)))
            .collect();
        self.textures.retain(|key, _| needed.contains(key));
        for settings in std::iter::once(solid)
            .chain(doc.strokes.iter().map(|s| s.brush.texture))
            .chain(self.repeat_preview.iter().map(|(s, _)| s.brush.texture))
        {
            let key = texture::key(settings);
            if !self.textures.contains_key(&key) {
                let bind = self.create_mask(settings);
                self.textures.insert(key, bind);
            }
        }
        let ids: HashSet<u64> = doc.strokes.iter().map(|s| s.id).collect();
        self.meshes.retain(|id, _| ids.contains(id));
        for stroke in doc.strokes.iter().filter(|s| doc.visible(s)) {
            let layer_opacity = doc.layer_opacity(stroke.layer_id);
            let image_tint_with_brush = stroke
                .brush
                .texture
                .image
                .is_none_or(|image| image.tint_with_brush);
            if self.meshes.get(&stroke.id).is_some_and(|m| {
                m.points == stroke.points.len()
                    && m.modifier_key
                        == crate::stroke_modifiers::GeometryCacheKey::for_stroke(stroke)
                    && m.brush_kind == stroke.brush.kind
                    && m.material == stroke.material
                    && m.image_tint_with_brush == image_tint_with_brush
                    && (m.layer_opacity - layer_opacity).abs() < 0.000_001
            }) {
                continue;
            }
            let mut vertices = geometry::stroke_mesh(stroke);
            if self.selected.contains(&stroke.id) {
                let color = crate::app::ui_style::canvas_selection_accent(
                    self.background,
                    self.screen_background.is_some(),
                )
                .to_array();
                let selection_color = [
                    color[0] as f32 / 255.0,
                    color[1] as f32 / 255.0,
                    color[2] as f32 / 255.0,
                ];
                for vertex in &mut vertices {
                    vertex.color = selection_color;
                }
            }
            if !image_tint_with_brush && !self.selected.contains(&stroke.id) {
                for vertex in &mut vertices {
                    vertex.color = [1.0; 3];
                }
            }
            for vertex in &mut vertices {
                vertex.opacity *= layer_opacity;
            }
            let data = bytemuck::cast_slice(&vertices);
            if let Some(mesh) = self
                .meshes
                .get_mut(&stroke.id)
                .filter(|m| m.capacity >= data.len())
            {
                if !data.is_empty() {
                    self.state.queue.write_buffer(&mesh.buffer, 0, data);
                }
                mesh.count = vertices.len() as u32;
                mesh.points = stroke.points.len();
                mesh.modifier_key = crate::stroke_modifiers::GeometryCacheKey::for_stroke(stroke);
                mesh.material = stroke.material;
                mesh.layer_opacity = layer_opacity;
                mesh.image_tint_with_brush = image_tint_with_brush;
                mesh.bounds = vertex_bounds(&vertices);
            } else {
                let mut mesh = self.mesh(
                    &vertices,
                    stroke.points.len(),
                    stroke.brush.kind,
                    stroke.material,
                );
                mesh.layer_opacity = layer_opacity;
                mesh.modifier_key = crate::stroke_modifiers::GeometryCacheKey::for_stroke(stroke);
                mesh.image_tint_with_brush = image_tint_with_brush;
                self.meshes.insert(stroke.id, mesh);
            }
        }
        let image_ids: HashSet<u64> = doc.image_planes.iter().map(|plane| plane.id).collect();
        self.image_plane_textures
            .retain(|id, _| image_ids.contains(id));
        self.image_plane_meshes
            .retain(|id, _| image_ids.contains(id));
        for image_plane in doc.image_planes.iter().filter(|plane| plane.visible) {
            if !self.image_plane_textures.contains_key(&image_plane.id) {
                continue;
            }
            if self
                .image_plane_meshes
                .get(&image_plane.id)
                .is_some_and(|mesh| mesh.plane == *image_plane)
            {
                continue;
            }
            let mesh = self.image_plane_mesh(image_plane);
            self.image_plane_meshes.insert(image_plane.id, mesh);
        }
        let reference_ids: HashSet<u64> =
            doc.model_references.iter().map(|model| model.id).collect();
        self.obj_reference_meshes
            .retain(|id, _| reference_ids.contains(id));
        if self.grid.as_ref().is_none_or(|g| g.0 != plane) {
            let mesh = self.mesh(
                &plane.vertices(),
                0,
                BrushKind::Round,
                StrokeMaterial::Shaded,
            );
            self.grid = Some((plane, mesh));
        }
        let settings = self.lighting;
        if settings.ground && self.ground.as_ref().is_none_or(|g| g.0 != settings) {
            let h = settings.ground_height;
            let [x, z] = settings.ground_center;
            let r = settings.ground_size * 0.5;
            let vertices: Vec<_> = [
                [x - r, h, z - r],
                [x + r, h, z - r],
                [x + r, h, z + r],
                [x - r, h, z - r],
                [x + r, h, z + r],
                [x - r, h, z + r],
            ]
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0.0, 1.0, 0.0],
                color: settings.ground_color,
                emission: [0.0; 3],
                uv: [0.0; 2],
                opacity: 1.0,
                material: 0.0,
                grain_uv: [0.0; 2],
                grain_dynamics: [1.0; 2],
                gradient_t: 0.0,
            })
            .collect();
            self.ground = Some((
                settings,
                self.mesh(&vertices, 0, BrushKind::Round, StrokeMaterial::Shaded),
            ));
        }
        let bounds = doc
            .strokes
            .iter()
            .filter(|s| doc.visible(s))
            .filter_map(|s| self.meshes.get(&s.id)?.bounds)
            .reduce(|(lo, hi), (min, max)| (lo.min(min), hi.max(max)));
        let shadow_enabled = settings.shadows && bounds.is_some();
        let fit = bounds.map(|(min, max)| lighting::shadow_fit(min, max, settings));
        let light = settings.direction();
        let uniform = SceneUniform {
            view: camera
                .matrix(size[0] as f32 / size[1] as f32)
                .to_cols_array(),
            light_view: fit
                .as_ref()
                .map_or(Mat4::IDENTITY, |f| f.matrix)
                .to_cols_array(),
            light: [light.x, light.y, light.z, settings.ambient],
            settings: [
                settings.intensity,
                f32::from(shadow_enabled),
                settings.strength,
                settings.bias / fit.as_ref().map_or(1.0, |f| f.depth_span),
            ],
            shadow: [
                1.0 / SHADOW_SIZE as f32,
                settings.bias,
                f32::from(self.sample_count() > 1),
                f32::from(self.toon_shading),
            ],
            camera: camera.eye().extend(self.grain.clamp(0.0, 0.12)).to_array(),
            background: [
                crate::presentation::srgb_to_linear(self.background[0]),
                crate::presentation::srgb_to_linear(self.background[1]),
                crate::presentation::srgb_to_linear(self.background[2]),
                1.0,
            ],
            resolution: [size[0] as f32, size[1] as f32, 0.0, 0.0],
        };
        self.state
            .queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let target = self.target.as_ref().unwrap();
        let screen_background = self.screen_background.as_ref().map(|background| {
            let vertices = screen_background_vertices(
                target.size,
                background.size,
                background.fit,
                background.alignment,
                background.opacity,
            );
            self.state
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Screen background image quad"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let mut encoder =
            self.state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Canvas frame"),
                });
        let mesh_refs: HashMap<_, _> = self.meshes.iter().map(|(id, mesh)| (*id, mesh)).collect();
        let clip_masks = self.record_layer_clip_masks(&mut encoder, doc, size, &mesh_refs);
        if shadow_enabled {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Directional shadow casters"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            for stroke in bottom_to_top(
                doc.strokes
                    .iter()
                    .filter(|s| {
                        doc.visible(s)
                            && !doc.layer_clip_source(s.layer_id).is_some()
                            && !is_translucent(
                                s,
                                doc.layer_opacity(s.layer_id),
                                doc.layer_blend_mode(s.layer_id),
                            )
                    })
                    .collect(),
                doc,
            ) {
                if let Some(mesh) = self.meshes.get(&stroke.id) {
                    pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                    pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw(0..mesh.count, 0..1);
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Canvas pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: target
                            .multisample_color
                            .as_ref()
                            .unwrap_or(if effects_enabled {
                                &target.scene_view
                            } else {
                                &target.view
                            }),
                        resolve_target: target.multisample_color.as_ref().map(|_| {
                            if effects_enabled {
                                &target.scene_view
                            } else {
                                &target.view
                            }
                        }),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: crate::presentation::srgb_to_linear(self.background[0]) as f64,
                                g: crate::presentation::srgb_to_linear(self.background[1]) as f64,
                                b: crate::presentation::srgb_to_linear(self.background[2]) as f64,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: target
                            .multisample_emission
                            .as_ref()
                            .unwrap_or(&target.emission_view),
                        resolve_target: target
                            .multisample_emission
                            .as_ref()
                            .map(|_| &target.emission_view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let (Some(background), Some(buffer)) = (&self.screen_background, &screen_background)
            {
                pass.set_pipeline(screen_background_pipeline);
                pass.set_bind_group(0, &background.bind, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_bind_group(1, &self.textures[&texture::key(solid)], &[]);
            pass.set_bind_group(2, &self.shadow_bind, &[]);
            // Ground, grid, and models share the stroke pipeline but have no grain.
            pass.set_bind_group(3, &self.textures[&texture::key(solid)], &[]);
            pass.set_bind_group(4, &self.clip_default_bind, &[]);
            if settings.ground {
                pass.set_pipeline(triangles);
                if let Some((_, mesh)) = &self.ground {
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw(0..mesh.count, 0..1);
                }
            }
            pass.set_pipeline(lines);
            if let Some((_, mesh)) = &self.grid {
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
            pass.set_pipeline(triangles);
            for model in doc.model_references.iter().filter(|model| model.visible) {
                if let Some(meshes) = self.obj_reference_meshes.get(&model.id) {
                    for batch in &meshes.opaque {
                        if let Some(texture) = &batch.texture {
                            pass.set_bind_group(1, &texture.bind, &[]);
                        } else {
                            pass.set_bind_group(1, &self.textures[&texture::key(solid)], &[]);
                        }
                        pass.set_vertex_buffer(0, batch.mesh.buffer.slice(..));
                        pass.draw(0..batch.mesh.count, 0..1);
                    }
                }
            }
            pass.set_pipeline(transparent_triangles);
            for model in doc.model_references.iter().filter(|model| model.visible) {
                if let Some(meshes) = self.obj_reference_meshes.get(&model.id) {
                    for batch in &meshes.translucent {
                        if let Some(texture) = &batch.texture {
                            pass.set_bind_group(1, &texture.bind, &[]);
                        } else {
                            pass.set_bind_group(1, &self.textures[&texture::key(solid)], &[]);
                        }
                        pass.set_vertex_buffer(0, batch.mesh.buffer.slice(..));
                        pass.draw(0..batch.mesh.count, 0..1);
                    }
                }
            }
            for stroke in bottom_to_top(
                doc.strokes
                    .iter()
                    .filter(|s| {
                        doc.visible(s)
                            && !is_translucent(
                                s,
                                doc.layer_opacity(s.layer_id),
                                doc.layer_blend_mode(s.layer_id),
                            )
                    })
                    .collect(),
                doc,
            ) {
                if let Some(mesh) = self.meshes.get(&stroke.id) {
                    pass.set_pipeline(if stroke.material == StrokeMaterial::Cutout {
                        cutout_triangles
                    } else if doc.layer_clip_source(stroke.layer_id).is_some() {
                        transparent_triangles
                    } else {
                        triangles
                    });
                    pass.set_bind_group(
                        4,
                        self.clip_binding(doc, stroke.layer_id, &clip_masks),
                        &[],
                    );
                    pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                    pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw(0..mesh.count, 0..1);
                }
            }
            for (stroke, mesh) in back_to_front(
                doc.strokes
                    .iter()
                    .filter(|s| {
                        doc.visible(s)
                            && is_translucent(
                                s,
                                doc.layer_opacity(s.layer_id),
                                doc.layer_blend_mode(s.layer_id),
                            )
                            && s.material != StrokeMaterial::Cutout
                    })
                    .filter_map(|s| self.meshes.get(&s.id).map(|m| (s, m)))
                    .collect(),
                camera,
                doc,
            ) {
                let mode = doc.layer_blend_mode(stroke.layer_id);
                let pipeline = if mode == LayerBlendMode::Normal {
                    transparent_triangles
                } else {
                    &self.blend_pipelines[&(mode, requested_samples)]
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(4, self.clip_binding(doc, stroke.layer_id, &clip_masks), &[]);
                pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
            for (stroke, mesh) in &self.repeat_preview {
                let mode = doc.layer_blend_mode(stroke.layer_id);
                let pipeline = if stroke.material == StrokeMaterial::Cutout {
                    &self.cutout_triangles
                } else if mode == LayerBlendMode::Normal {
                    &self.transparent_triangles
                } else {
                    &self.blend_pipelines[&(mode, requested_samples)]
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(4, self.clip_binding(doc, stroke.layer_id, &clip_masks), &[]);
                pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
            pass.set_pipeline(image_planes_pipeline);
            let mut image_planes: Vec<_> = doc
                .image_planes
                .iter()
                .filter(|plane| plane.visible && self.image_plane_textures.contains_key(&plane.id))
                .filter_map(|plane| {
                    self.image_plane_meshes
                        .get(&plane.id)
                        .map(|mesh| (plane, mesh))
                })
                .collect();
            image_planes.sort_by(|(a, _), (b, _)| {
                (a.origin - camera.target)
                    .dot(camera.basis().2)
                    .total_cmp(&(b.origin - camera.target).dot(camera.basis().2))
                    .then_with(|| a.id.cmp(&b.id))
            });
            for (plane, mesh) in image_planes {
                pass.set_bind_group(1, &self.image_plane_textures[&plane.id].bind, &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
        if effects_enabled {
            self.record_post_effects(
                &mut encoder,
                &target.scene_view,
                &target.emission_view,
                &target.depth,
                &target.view,
                size,
                camera,
                None,
            );
        }
        self.state.queue.submit(Some(encoder.finish()));
        target.id
    }

    /// Render clean artwork into an independent target for PNG export.
    pub fn render_png_pixels(
        &mut self,
        doc: &Document,
        camera: &Camera,
        size: [u32; 2],
        background: [f32; 3],
        settings: Lighting,
    ) -> Result<Vec<u8>, String> {
        self.render_artwork_pixels(doc, camera, size, Some(background), settings, false)
            .map(|(pixels, _)| pixels)
    }

    /// Render artwork and, when the adapter supports timestamp queries, return
    /// the GPU time from the scene pass through the final post-effect pass.
    pub fn render_png_pixels_with_gpu_time(
        &mut self,
        doc: &Document,
        camera: &Camera,
        size: [u32; 2],
        background: [f32; 3],
        settings: Lighting,
    ) -> Result<(Vec<u8>, Option<std::time::Duration>), String> {
        self.render_artwork_pixels(doc, camera, size, Some(background), settings, true)
    }

    /// Render artwork over transparency, excluding image planes and the ground.
    pub fn render_transparent_png_pixels(
        &mut self,
        doc: &Document,
        camera: &Camera,
        size: [u32; 2],
        settings: Lighting,
    ) -> Result<Vec<u8>, String> {
        self.render_artwork_pixels(doc, camera, size, None, settings, false)
            .map(|(pixels, _)| pixels)
    }

    fn render_artwork_pixels(
        &mut self,
        doc: &Document,
        camera: &Camera,
        size: [u32; 2],
        background: Option<[f32; 3]>,
        settings: Lighting,
        measure_gpu_time: bool,
    ) -> Result<(Vec<u8>, Option<std::time::Duration>), String> {
        self.prepare_tip_textures(doc);
        let limit = self.state.device.limits().max_texture_dimension_2d;
        let pixel_count = (size[0] as u64)
            .checked_mul(size[1] as u64)
            .ok_or("PNG dimensions overflow")?;
        if size.contains(&0)
            || size
                .iter()
                .any(|dimension| *dimension > limit || *dimension > 8192)
            || pixel_count > 8_388_608
        {
            return Err(
                "Image dimensions exceed the 8192-pixel or 8-megapixel export limit".into(),
            );
        }
        if background.is_some_and(|color| {
            !color
                .iter()
                .all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel))
        }) {
            return Err("Canvas background must contain finite RGB values from 0 to 1".into());
        }

        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let effects_enabled = self.post_effects.glow > 0.0
            || self.post_effects.depth_of_field > 0.0
            || self.post_effects.pixel_size > 1.0;
        let (
            triangles,
            cutout_triangles,
            coverage_triangles,
            transparent_triangles,
            image_planes_pipeline,
        ) = if effects_enabled {
            (
                &self.single_triangles,
                &self.single_cutout_triangles,
                &self.single_coverage_triangles,
                &self.single_transparent_triangles,
                &self.single_image_planes_pipeline,
            )
        } else {
            (
                &self.triangles,
                &self.cutout_triangles,
                &self.coverage_triangles,
                &self.transparent_triangles,
                &self.image_planes_pipeline,
            )
        };
        let screen_background_pipeline = if effects_enabled {
            &self.single_screen_background_pipeline
        } else {
            &self.screen_background_pipeline
        };
        let sample_count = if effects_enabled {
            1
        } else {
            canvas_sample_count(&self.state.adapter)
        };
        let color = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Artwork PNG export target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&Default::default());
        let scene_color = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Artwork PNG post-processing source"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let scene_view = scene_color.create_view(&Default::default());
        let emission_texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Artwork PNG emissive contribution"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let emission_view = emission_texture.create_view(&Default::default());
        let multisample_color = (sample_count > 1).then(|| {
            self.state
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Artwork PNG multisample color"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let multisample_emission = (sample_count > 1).then(|| {
            self.state
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Artwork PNG multisample emissive contribution"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let depth_texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Artwork PNG depth"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if effects_enabled {
                    wgpu::TextureUsages::TEXTURE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        });
        let depth = depth_texture.create_view(&Default::default());

        let solid = TextureSettings::default();
        for texture_settings in std::iter::once(solid).chain(
            doc.strokes
                .iter()
                .filter(|stroke| doc.visible(stroke))
                .map(|s| s.brush.texture),
        ) {
            let key = texture::key(texture_settings);
            if !self.textures.contains_key(&key) {
                let bind = self.create_mask(texture_settings);
                self.textures.insert(key, bind);
            }
        }
        let meshes: Vec<_> = doc
            .strokes
            .iter()
            .filter(|stroke| doc.visible(stroke))
            .map(|stroke| {
                let vertices = geometry::stroke_mesh(stroke);
                (
                    stroke,
                    self.layer_mesh(
                        &vertices,
                        stroke.points.len(),
                        stroke.brush.kind,
                        stroke.material,
                        doc.layer_opacity(stroke.layer_id),
                    ),
                )
            })
            .collect();
        let image_plane_meshes: Vec<_> = if background.is_some() {
            doc.image_planes
                .iter()
                .filter(|plane| plane.visible && self.image_plane_textures.contains_key(&plane.id))
                .map(|plane| (plane, self.image_plane_mesh(plane)))
                .collect()
        } else {
            Vec::new()
        };
        let bounds = meshes
            .iter()
            .filter_map(|(_, mesh)| mesh.bounds)
            .reduce(|(lo, hi), (min, max)| (lo.min(min), hi.max(max)));
        let shadow_enabled = settings.shadows && bounds.is_some();
        let fit = bounds.map(|(min, max)| lighting::shadow_fit(min, max, settings));
        let light = settings.direction();
        let uniform = SceneUniform {
            view: camera
                .matrix(size[0] as f32 / size[1] as f32)
                .to_cols_array(),
            light_view: fit
                .as_ref()
                .map_or(Mat4::IDENTITY, |f| f.matrix)
                .to_cols_array(),
            light: [light.x, light.y, light.z, settings.ambient],
            settings: [
                settings.intensity,
                f32::from(shadow_enabled),
                settings.strength,
                settings.bias / fit.as_ref().map_or(1.0, |f| f.depth_span),
            ],
            shadow: [
                1.0 / SHADOW_SIZE as f32,
                settings.bias,
                f32::from(sample_count > 1),
                f32::from(self.toon_shading),
            ],
            camera: camera.eye().extend(self.grain.clamp(0.0, 0.12)).to_array(),
            background: background.map_or([0.0, 0.0, 0.0, 0.0], |color| {
                [
                    crate::presentation::srgb_to_linear(color[0]),
                    crate::presentation::srgb_to_linear(color[1]),
                    crate::presentation::srgb_to_linear(color[2]),
                    1.0,
                ]
            }),
            resolution: [size[0] as f32, size[1] as f32, 0.0, 0.0],
        };
        self.state
            .queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let screen_background = background
            .and(self.screen_background.as_ref())
            .map(|image| {
                let vertices = screen_background_vertices(
                    size,
                    image.size,
                    image.fit,
                    image.alignment,
                    image.opacity,
                );
                self.state
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Artwork screen background image quad"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
            });
        let mut encoder =
            self.state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Artwork PNG export"),
                });
        let mesh_refs: HashMap<_, _> = meshes
            .iter()
            .map(|(stroke, mesh)| (stroke.id, mesh))
            .collect();
        let clip_masks = self.record_layer_clip_masks(&mut encoder, doc, size, &mesh_refs);
        let timestamp_queries = (measure_gpu_time
            && self
                .state
                .device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY))
        .then(|| {
            self.state
                .device
                .create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("Post-effects GPU timing"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                })
        });
        if shadow_enabled {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Artwork PNG shadows"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            for (stroke, mesh) in bottom_to_top_meshes(
                meshes
                    .iter()
                    .filter(|(stroke, _)| {
                        !is_translucent(
                            stroke,
                            doc.layer_opacity(stroke.layer_id),
                            doc.layer_blend_mode(stroke.layer_id),
                        )
                    })
                    .map(|(stroke, mesh)| (*stroke, mesh))
                    .collect(),
                doc,
            ) {
                pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Artwork PNG color"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: multisample_color.as_ref().unwrap_or(if effects_enabled {
                            &scene_view
                        } else {
                            &color_view
                        }),
                        resolve_target: multisample_color.as_ref().map(|_| {
                            if effects_enabled {
                                &scene_view
                            } else {
                                &color_view
                            }
                        }),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(background.map_or(
                                wgpu::Color::TRANSPARENT,
                                |background| wgpu::Color {
                                    r: crate::presentation::srgb_to_linear(background[0]) as f64,
                                    g: crate::presentation::srgb_to_linear(background[1]) as f64,
                                    b: crate::presentation::srgb_to_linear(background[2]) as f64,
                                    a: 1.0,
                                },
                            )),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: multisample_emission.as_ref().unwrap_or(&emission_view),
                        resolve_target: multisample_emission.as_ref().map(|_| &emission_view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamp_queries.as_ref().map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: (!effects_enabled).then_some(1),
                    }
                }),
                occlusion_query_set: None,
            });
            if let (Some(image), Some(buffer)) = (&self.screen_background, &screen_background) {
                pass.set_pipeline(screen_background_pipeline);
                pass.set_bind_group(0, &image.bind, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_bind_group(1, &self.textures[&texture::key(solid)], &[]);
            pass.set_bind_group(2, &self.shadow_bind, &[]);
            // Offscreen ground draws also require the pipeline's grain binding.
            pass.set_bind_group(3, &self.textures[&texture::key(solid)], &[]);
            pass.set_bind_group(4, &self.clip_default_bind, &[]);
            if background.is_some() && settings.ground {
                pass.set_pipeline(triangles);
                if let Some((cached_settings, ground)) = &self.ground {
                    if *cached_settings == settings {
                        pass.set_vertex_buffer(0, ground.buffer.slice(..));
                        pass.draw(0..ground.count, 0..1);
                    }
                }
            }
            for (stroke, mesh) in meshes.iter().filter(|(stroke, _)| {
                !is_translucent(
                    stroke,
                    doc.layer_opacity(stroke.layer_id),
                    doc.layer_blend_mode(stroke.layer_id),
                )
            }) {
                pass.set_pipeline(if stroke.material == StrokeMaterial::Cutout {
                    cutout_triangles
                } else if background.is_none() {
                    coverage_triangles
                } else if doc.layer_clip_source(stroke.layer_id).is_some() {
                    transparent_triangles
                } else {
                    triangles
                });
                pass.set_bind_group(4, self.clip_binding(doc, stroke.layer_id, &clip_masks), &[]);
                pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
            for (stroke, mesh) in back_to_front(
                meshes
                    .iter()
                    .filter(|(stroke, _)| {
                        is_translucent(
                            stroke,
                            doc.layer_opacity(stroke.layer_id),
                            doc.layer_blend_mode(stroke.layer_id),
                        ) && stroke.material != StrokeMaterial::Cutout
                    })
                    .map(|(stroke, mesh)| (*stroke, mesh))
                    .collect(),
                camera,
                doc,
            ) {
                let mode = doc.layer_blend_mode(stroke.layer_id);
                let mode = if background.is_none() && mode == LayerBlendMode::Multiply {
                    LayerBlendMode::Normal
                } else {
                    mode
                };
                let pipeline = if mode == LayerBlendMode::Normal {
                    transparent_triangles
                } else {
                    &self.blend_pipelines[&(mode, sample_count)]
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(4, self.clip_binding(doc, stroke.layer_id, &clip_masks), &[]);
                pass.set_bind_group(1, self.brush_texture(stroke), &[]);
                pass.set_bind_group(3, self.grain_texture(stroke), &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
            pass.set_pipeline(image_planes_pipeline);
            let mut image_planes: Vec<_> = image_plane_meshes.iter().collect();
            image_planes.sort_by(|(a, _), (b, _)| {
                (a.origin - camera.target)
                    .dot(camera.basis().2)
                    .total_cmp(&(b.origin - camera.target).dot(camera.basis().2))
                    .then_with(|| a.id.cmp(&b.id))
            });
            for (plane, mesh) in image_planes {
                pass.set_bind_group(1, &self.image_plane_textures[&plane.id].bind, &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
        if effects_enabled {
            self.record_post_effects(
                &mut encoder,
                &scene_view,
                &emission_view,
                &depth,
                &color_view,
                size,
                camera,
                timestamp_queries.as_ref(),
            );
        }
        let row = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Artwork PNG readback"),
            size: (row as u64) * size[1] as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size[1]),
                },
            },
            extent,
        );
        let timestamp_readback = timestamp_queries.as_ref().map(|queries| {
            let resolved = self.state.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Post-effects GPU timing resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = self.state.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Post-effects GPU timing readback"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.resolve_query_set(queries, 0..2, &resolved, 0);
            encoder.copy_buffer_to_buffer(&resolved, 0, &readback, 0, 16);
            (resolved, readback)
        });
        self.state.queue.submit(Some(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.state.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let mapped = buffer.slice(..).get_mapped_range();
        let mut pixels: Vec<u8> = mapped
            .chunks(row as usize)
            .flat_map(|bytes| bytes[..size[0] as usize * 4].iter().copied())
            .collect();
        drop(mapped);
        buffer.unmap();
        if background.is_none() {
            unpremultiply_rgba(&mut pixels);
        }
        let gpu_duration = if let Some((resolved, readback)) = timestamp_readback {
            let (sender, receiver) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            self.state.device.poll(wgpu::Maintain::Wait);
            receiver
                .recv()
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            let mapped = readback.slice(..).get_mapped_range();
            let values: &[u64] = bytemuck::cast_slice(&mapped);
            let ticks = values[1].saturating_sub(values[0]);
            let nanos = ticks as f64 * f64::from(self.state.queue.get_timestamp_period());
            drop(mapped);
            readback.unmap();
            drop(resolved);
            Some(std::time::Duration::from_nanos(
                nanos.min(u64::MAX as f64) as u64
            ))
        } else {
            None
        };
        Ok((pixels, gpu_duration))
    }

    /// Verification-only readback; normal frames never wait for GPU pixels.
    pub fn verification_pixels(&self) -> Vec<u8> {
        let target = self.target.as_ref().unwrap();
        let row = (target.size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Verification readback"),
            size: (row * target.size[1]) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .state
            .device
            .create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(target.size[1]),
                },
            },
            wgpu::Extent3d {
                width: target.size[0],
                height: target.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.state.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        self.state.device.poll(wgpu::Maintain::Wait);
        receive.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        let pixels = mapped
            .chunks(row as usize)
            .flat_map(|bytes| bytes[..target.size[0] as usize * 4].iter().copied())
            .collect();
        drop(mapped);
        buffer.unmap();
        pixels
    }
}
impl Drop for SceneRenderer {
    fn drop(&mut self) {
        if let Some(target) = self.target.take() {
            self.state.renderer.write().free_texture(&target.id);
        }
    }
}

fn vertex_bounds(vertices: &[Vertex]) -> Option<(Vec3, Vec3)> {
    vertices
        .iter()
        .map(|v| Vec3::from(v.position))
        .map(|p| (p, p))
        .reduce(|(lo, hi), (min, max)| (lo.min(min), hi.max(max)))
}

#[cfg(test)]
mod screen_background_tests {
    use super::*;

    #[test]
    fn screen_background_fit_and_fill_keep_alignment_uv_and_opacity() {
        let fit = screen_background_vertices(
            [1600, 900],
            [1000, 1000],
            BackgroundImageFit::Fit,
            [0.5, 0.5],
            0.4,
        );
        assert!((fit[0].position[0] + 0.5625).abs() < 1e-6);
        assert!((fit[1].position[0] - 0.5625).abs() < 1e-6);
        assert_eq!(fit[0].uv, [0.0, 0.0]);
        assert_eq!(fit[2].uv, [1.0, 1.0]);
        assert!(fit.iter().all(|vertex| vertex.opacity == 0.4));

        let fill = screen_background_vertices(
            [1600, 900],
            [1000, 1000],
            BackgroundImageFit::Fill,
            [0.5, 0.0],
            0.7,
        );
        assert_eq!(fill[0].position, [-1.0, 1.0]);
        assert_eq!(fill[2].position, [1.0, -1.0]);
        assert!((fill[0].uv[1] - 0.0).abs() < 1e-6);
        assert!((fill[2].uv[1] - 0.5625).abs() < 1e-6);
        assert!(fill.iter().all(|vertex| vertex.opacity == 0.7));

        let right_aligned_fill = screen_background_vertices(
            [900, 1600],
            [1600, 900],
            BackgroundImageFit::Fill,
            [1.0, 0.5],
            1.0,
        );
        assert!((right_aligned_fill[0].uv[0] - 0.68359375).abs() < 1e-6);
        assert!((right_aligned_fill[1].uv[0] - 1.0).abs() < 1e-6);
    }
}
