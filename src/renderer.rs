use crate::lighting::{self, Lighting, SHADOW_SIZE};
use crate::surface::GuidePreview;
use crate::texture::{self, TextureKind, TextureSettings};
use crate::{
    document::Document,
    geometry::{self, Camera, Vertex},
};
use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use glam::{Mat4, Vec3};
use std::collections::{HashMap, HashSet};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniform {
    view: [f32; 16],
    light_view: [f32; 16],
    light: [f32; 4],
    settings: [f32; 4],
    shadow: [f32; 4],
}
struct CanvasTarget {
    color: wgpu::Texture,
    view: wgpu::TextureView,
    multisample_color: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    id: egui::TextureId,
    size: [u32; 2],
    sample_count: u32,
}

struct Mesh {
    buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
    points: usize,
    bounds: Option<(Vec3, Vec3)>,
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

pub struct SceneRenderer {
    state: egui_wgpu::RenderState,
    triangles: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    textures: HashMap<(TextureKind, u32, u32), wgpu::BindGroup>,
    meshes: HashMap<u64, Mesh>,
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
            vertex:wgpu::VertexState {module:&shadow_shader,entry_point:Some("vs_shadow"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout {array_stride:std::mem::size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2]}]},
            fragment:Some(wgpu::FragmentState {module:&shadow_shader,entry_point:Some("fs_shadow"),compilation_options:Default::default(),targets:&[]}),
            primitive:wgpu::PrimitiveState {cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:true,depth_compare:wgpu::CompareFunction::LessEqual,stencil:Default::default(),bias:wgpu::DepthBiasState {constant:1,slope_scale:1.0,clamp:0.0}}),
            multisample:Default::default(),multiview:None,cache:None,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout, &texture_layout, &shadow_layout],
            push_constant_ranges: &[],
        });
        let sample_count = canvas_sample_count(&state.adapter);
        let create = |topology| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("3D strokes"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs_main"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout {array_stride:std::mem::size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2]}]},
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fs_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba8UnormSrgb,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),
            primitive:wgpu::PrimitiveState {topology,cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:topology != wgpu::PrimitiveTopology::LineList,depth_compare:wgpu::CompareFunction::LessEqual,stencil:Default::default(),bias:Default::default()}),
            multisample:wgpu::MultisampleState {count:sample_count,mask:!0,alpha_to_coverage_enabled:sample_count > 1},multiview:None,cache:None,
        })
        };
        let triangles = create(wgpu::PrimitiveTopology::TriangleList);
        let lines = create(wgpu::PrimitiveTopology::LineList);
        let info = state.adapter.get_info();
        let adapter = format!("{} / {:?}", info.name, info.backend);
        Self {
            state,
            triangles,
            lines,
            uniform,
            bind,
            texture_layout,
            textures: HashMap::new(),
            meshes: HashMap::new(),
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
    pub fn invalidate(&mut self) {
        self.meshes.clear();
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
    pub fn set_repeat_preview(&mut self, strokes: &[crate::document::Stroke]) {
        if self
            .repeat_preview
            .iter()
            .map(|(stroke, _)| stroke)
            .eq(strokes.iter())
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
                (stroke.clone(), self.mesh(&vertices, stroke.points.len()))
            })
            .collect();
    }
    fn mesh(&self, vertices: &[Vertex], points: usize) -> Mesh {
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
            bounds: vertex_bounds(vertices),
        }
    }
    pub fn render(
        &mut self,
        doc: &Document,
        camera: &Camera,
        plane: GuidePreview,
        size: [u32; 2],
    ) -> egui::TextureId {
        let limit = self.state.device.limits().max_texture_dimension_2d;
        let size = [size[0].clamp(1, limit), size[1].clamp(1, limit)];
        if self.target.as_ref().is_none_or(|t| t.size != size) {
            if let Some(target) = self.target.take() {
                self.state.renderer.write().free_texture(&target.id);
            }
            let extent = wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            };
            let sample_count = canvas_sample_count(&self.state.adapter);
            let color = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D canvas"),
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
            let depth = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas depth"),
                size: extent,
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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
                multisample_color,
                depth: depth_view,
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
            if self
                .meshes
                .get(&stroke.id)
                .is_some_and(|m| m.points == stroke.points.len())
            {
                continue;
            }
            let mut vertices = geometry::stroke_mesh(stroke);
            if self.selected.contains(&stroke.id) {
                for vertex in &mut vertices {
                    vertex.color = [1.0, 0.67, 0.18];
                }
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
                mesh.bounds = vertex_bounds(&vertices);
            } else {
                let mesh = self.mesh(&vertices, stroke.points.len());
                self.meshes.insert(stroke.id, mesh);
            }
        }
        if self.grid.as_ref().is_none_or(|g| g.0 != plane) {
            let mesh = self.mesh(&plane.vertices(), 0);
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
                uv: [0.0; 2],
            })
            .collect();
            self.ground = Some((settings, self.mesh(&vertices, 0)));
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
                0.0,
            ],
        };
        self.state
            .queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let target = self.target.as_ref().unwrap();
        let mut encoder =
            self.state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Canvas frame"),
                });
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
            for stroke in doc.strokes.iter().filter(|s| doc.visible(s)) {
                if let Some(mesh) = self.meshes.get(&stroke.id) {
                    pass.set_bind_group(
                        1,
                        &self.textures[&texture::key(stroke.brush.texture)],
                        &[],
                    );
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw(0..mesh.count, 0..1);
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Canvas pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.multisample_color.as_ref().unwrap_or(&target.view),
                    resolve_target: target.multisample_color.as_ref().map(|_| &target.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: crate::presentation::srgb_to_linear(self.background[0]) as f64,
                            g: crate::presentation::srgb_to_linear(self.background[1]) as f64,
                            b: crate::presentation::srgb_to_linear(self.background[2]) as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
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
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_bind_group(1, &self.textures[&texture::key(solid)], &[]);
            pass.set_bind_group(2, &self.shadow_bind, &[]);
            if settings.ground {
                pass.set_pipeline(&self.triangles);
                if let Some((_, mesh)) = &self.ground {
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw(0..mesh.count, 0..1);
                }
            }
            pass.set_pipeline(&self.lines);
            if let Some((_, mesh)) = &self.grid {
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
            pass.set_pipeline(&self.triangles);
            for stroke in doc.strokes.iter().filter(|s| doc.visible(s)) {
                if let Some(mesh) = self.meshes.get(&stroke.id) {
                    pass.set_bind_group(
                        1,
                        &self.textures[&texture::key(stroke.brush.texture)],
                        &[],
                    );
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw(0..mesh.count, 0..1);
                }
            }
            for (stroke, mesh) in &self.repeat_preview {
                pass.set_bind_group(1, &self.textures[&texture::key(stroke.brush.texture)], &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.count, 0..1);
            }
        }
        self.state.queue.submit(Some(encoder.finish()));
        target.id
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
