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
use std::collections::{HashMap, HashSet};
use wgpu::util::DeviceExt;

struct Mesh {
    buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
    points: usize,
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
    selected: HashSet<u64>,
    grid: Option<(GuidePreview, Mesh)>,
    target: Option<(
        wgpu::TextureView,
        wgpu::TextureView,
        egui::TextureId,
        [u32; 2],
    )>,
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
            label: Some("Camera matrix"),
            contents: bytemuck::cast_slice(&glam::Mat4::IDENTITY.to_cols_array()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout, &texture_layout],
            push_constant_ranges: &[],
        });
        let create = |topology| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("3D strokes"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs_main"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout {array_stride:std::mem::size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x2]}]},
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fs_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba8UnormSrgb,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),
            primitive:wgpu::PrimitiveState {topology,cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:topology != wgpu::PrimitiveTopology::LineList,depth_compare:wgpu::CompareFunction::LessEqual,stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),multiview:None,cache:None,
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
            selected: HashSet::new(),
            grid: None,
            target: None,
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
        if self.target.as_ref().is_none_or(|t| t.3 != size) {
            if let Some((_, _, id, _)) = self.target.take() {
                self.state.renderer.write().free_texture(&id);
            }
            let extent = wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            };
            let color = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D canvas"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let depth = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
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
            self.target = Some((view, depth_view, id, size));
        }
        let solid = TextureSettings::default();
        let needed: HashSet<_> = doc
            .strokes
            .iter()
            .map(|s| texture::key(s.brush.texture))
            .chain(std::iter::once(texture::key(solid)))
            .collect();
        self.textures.retain(|key, _| needed.contains(key));
        for settings in std::iter::once(solid).chain(doc.strokes.iter().map(|s| s.brush.texture)) {
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
            } else {
                let mesh = self.mesh(&vertices, stroke.points.len());
                self.meshes.insert(stroke.id, mesh);
            }
        }
        if self.grid.as_ref().is_none_or(|g| g.0 != plane) {
            let mesh = self.mesh(&plane.vertices(), 0);
            self.grid = Some((plane, mesh));
        }
        self.state.queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::cast_slice(
                &camera
                    .matrix(size[0] as f32 / size[1] as f32)
                    .to_cols_array(),
            ),
        );
        let (view, depth, id, _) = self.target.as_ref().unwrap();
        let mut encoder =
            self.state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Canvas frame"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Canvas pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.018,
                            g: 0.029,
                            b: 0.046,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
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
        }
        self.state.queue.submit(Some(encoder.finish()));
        *id
    }
}
impl Drop for SceneRenderer {
    fn drop(&mut self) {
        if let Some((_, _, id, _)) = self.target.take() {
            self.state.renderer.write().free_texture(&id);
        }
    }
}
