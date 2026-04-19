use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use std::mem::size_of;
use wgpu::util::DeviceExt;

const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const LIGHTING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const INSTANCE_COUNT: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    models: [[[f32; 4]; 4]; INSTANCE_COUNT as usize],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightUniform {
    direction: [f32; 4],
    color_intensity: [f32; 4],
}

struct PassAttachments {
    _albedo: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    _normal: wgpu::Texture,
    normal_view: wgpu::TextureView,
    _lit: wgpu::Texture,
    lit_view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

struct FallbackInput {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct Example {
    graph: wgpu::RenderGraph,
    active_subpass_mask: wgpu::ActiveSubpassMask,
    surface_size: [u32; 2],
    frame_index: u64,
    pass_attachments: PassAttachments,
    fallback_input: FallbackInput,
    scene: SceneUniform,
    scene_buffer: wgpu::Buffer,
    light: LightUniform,
    light_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    geometry_bind_group: wgpu::BindGroup,
    lighting_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    geometry_pipeline: wgpu::RenderPipeline,
    lighting_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
}

impl Example {
    fn create_graph(output_format: wgpu::TextureFormat) -> wgpu::RenderGraph {
        let mut builder = wgpu::RenderGraphBuilder::new();
        builder.sample_count(1);
        let albedo = builder.add_transient_color("albedo", ALBEDO_FORMAT);
        let normal = builder.add_transient_color("normal", NORMAL_FORMAT);
        let lit = builder.add_transient_color("lit_hdr", LIGHTING_FORMAT);
        let output = builder.add_persistent_color("output", output_format);
        let depth = builder.add_persistent_depth("depth", DEPTH_FORMAT);
        let _ = builder
            .add_subpass("geometry")
            .writes_color(albedo)
            .writes_color(normal)
            .writes_depth(depth);
        let _ = builder
            .add_subpass("lighting")
            .reads(albedo)
            .reads(normal)
            .writes_color(lit);
        let _ = builder
            .add_subpass("composite")
            .reads(lit)
            .writes_color(output);
        builder.build().expect("deferred render graph build failed")
    }

    fn create_pass_attachments(
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
    ) -> PassAttachments {
        let extent = wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        let color_usage =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let albedo = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deferred_rendering albedo"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ALBEDO_FORMAT,
            usage: color_usage,
            view_formats: &[],
        });
        let normal = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deferred_rendering normal"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: NORMAL_FORMAT,
            usage: color_usage,
            view_formats: &[],
        });
        let lit = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deferred_rendering lit"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LIGHTING_FORMAT,
            usage: color_usage,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deferred_rendering depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        PassAttachments {
            albedo_view: albedo.create_view(&wgpu::TextureViewDescriptor::default()),
            normal_view: normal.create_view(&wgpu::TextureViewDescriptor::default()),
            lit_view: lit.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            _albedo: albedo,
            _normal: normal,
            _lit: lit,
            _depth: depth,
        }
    }

    fn create_fallback_input(device: &wgpu::Device, queue: &wgpu::Queue) -> FallbackInput {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deferred_rendering fallback_input"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ALBEDO_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &[0, 0, 0, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        FallbackInput {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            _texture: texture,
        }
    }

    fn create_checker_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        const SIZE: u32 = 4;
        let mut texels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let light_square = ((x + y) & 1) == 0;
                let (r, g, b) = if light_square {
                    (223, 188, 128)
                } else {
                    (65, 91, 171)
                };
                texels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("deferred_rendering checker"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ALBEDO_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * SIZE),
                rows_per_image: Some(SIZE),
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn create_subpass_bind_groups(
        device: &wgpu::Device,
        lighting_pipeline: &wgpu::RenderPipeline,
        composite_pipeline: &wgpu::RenderPipeline,
        fallback_input: &FallbackInput,
        light_buffer: &wgpu::Buffer,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        // Input-attachment reads come from render-pass state on current supported backends.
        // These bindings satisfy shader validation when no sampled fallback path is active.
        let sampled_fallback = &fallback_input.view;
        let lighting_bgl = lighting_pipeline.get_bind_group_layout(0);
        let lighting_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("deferred_rendering lighting_bind_group"),
            layout: &lighting_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(sampled_fallback),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(sampled_fallback),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: light_buffer.as_entire_binding(),
                },
            ],
        });

        let composite_bgl = composite_pipeline.get_bind_group_layout(0);
        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("deferred_rendering composite_bind_group"),
            layout: &composite_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(sampled_fallback),
            }],
        });
        (lighting_bind_group, composite_bind_group)
    }

    fn update_uniforms(&mut self, queue: &wgpu::Queue) {
        let time = self.frame_index as f32 * (1.0 / 60.0);
        let aspect = self.surface_size[0] as f32 / self.surface_size[1] as f32;

        let camera_angle = time * 0.35;
        let camera_position = Vec3::new(camera_angle.cos() * 6.0, 2.75, camera_angle.sin() * 6.0);
        let view = Mat4::look_at_rh(camera_position, Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
        let projection = Mat4::perspective_rh(45f32.to_radians(), aspect, 0.1, 100.0);
        self.scene.view_proj = (projection * view).to_cols_array_2d();

        let model_a = Mat4::from_translation(Vec3::new(-1.5, 0.0, 0.0))
            * Mat4::from_quat(Quat::from_rotation_y(time * 0.8));
        let model_b = Mat4::from_translation(Vec3::new(1.5, 0.0, 0.0))
            * Mat4::from_quat(Quat::from_euler(
                glam::EulerRot::XYZ,
                time * 0.45,
                -time * 0.6,
                0.0,
            ));
        self.scene.models = [model_a.to_cols_array_2d(), model_b.to_cols_array_2d()];

        self.light.direction = [(time * 0.9).cos(), -0.7, (time * 0.9).sin(), 0.0];
        self.light.color_intensity = [1.0, 0.96, 0.88, 2.25];

        queue.write_buffer(&self.scene_buffer, 0, bytemuck::bytes_of(&self.scene));
        queue.write_buffer(&self.light_buffer, 0, bytemuck::bytes_of(&self.light));
    }

    fn recreate_runtime_targets(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
    ) {
        self.pass_attachments = Self::create_pass_attachments(config, device);
        (self.lighting_bind_group, self.composite_bind_group) = Self::create_subpass_bind_groups(
            device,
            &self.lighting_pipeline,
            &self.composite_pipeline,
            &self.fallback_input,
            &self.light_buffer,
        );
    }
}

impl crate::framework::Example for Example {
    const SRGB: bool = false;

    fn required_features() -> wgpu::Features {
        wgpu::Features::MULTI_SUBPASS | wgpu::Features::TRANSIENT_ATTACHMENTS
    }

    fn required_limits() -> wgpu::Limits {
        wgpu::Limits {
            max_subpasses: 3,
            max_subpass_color_attachments: 4,
            max_input_attachments: 2,
            ..wgpu::Limits::downlevel_defaults()
        }
    }

    fn init(
        config: &wgpu::SurfaceConfiguration,
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        let graph = Self::create_graph(config.view_formats[0]);
        let active_subpass_mask = graph
            .resolve_active(&[
                wgpu::SubpassIndex(0),
                wgpu::SubpassIndex(1),
                wgpu::SubpassIndex(2),
            ])
            .expect("deferred active subpass mask");
        let color_attachment_formats = [
            Some(ALBEDO_FORMAT),
            Some(NORMAL_FORMAT),
            Some(LIGHTING_FORMAT),
            Some(config.view_formats[0]),
        ];

        let geometry_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deferred_rendering geometry_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("geometry.wgsl").into()),
        });
        let lighting_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deferred_rendering lighting_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lighting.wgsl").into()),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deferred_rendering composite_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });

        let geometry_target =
            graph.make_subpass_target(0, &color_attachment_formats, Some(DEPTH_FORMAT));
        let lighting_target =
            graph.make_subpass_target(1, &color_attachment_formats, Some(DEPTH_FORMAT));
        let composite_target =
            graph.make_subpass_target(2, &color_attachment_formats, Some(DEPTH_FORMAT));

        let geometry_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("deferred_rendering geometry_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &geometry_shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &geometry_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(ALBEDO_FORMAT.into()), Some(NORMAL_FORMAT.into())],
            }),
            multiview_mask: None,
            subpass_target: Some(geometry_target),
            cache: None,
        });
        let lighting_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("deferred_rendering lighting_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &lighting_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &lighting_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(LIGHTING_FORMAT.into())],
            }),
            multiview_mask: None,
            subpass_target: Some(lighting_target),
            cache: None,
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("deferred_rendering composite_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(config.view_formats[0].into())],
            }),
            multiview_mask: None,
            subpass_target: Some(composite_target),
            cache: None,
        });

        let checker_texture_view = Self::create_checker_texture(device, queue);
        let checker_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("deferred_rendering checker_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let scene = SceneUniform::zeroed();
        let scene_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("deferred_rendering scene_uniforms"),
            contents: bytemuck::bytes_of(&scene),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let light = LightUniform::zeroed();
        let light_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("deferred_rendering light_uniforms"),
            contents: bytemuck::bytes_of(&light),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let geometry_bgl = geometry_pipeline.get_bind_group_layout(0);
        let geometry_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("deferred_rendering geometry_bind_group"),
            layout: &geometry_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&checker_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&checker_sampler),
                },
            ],
        });

        let pass_attachments = Self::create_pass_attachments(config, device);
        let fallback_input = Self::create_fallback_input(device, queue);
        let (lighting_bind_group, composite_bind_group) = Self::create_subpass_bind_groups(
            device,
            &lighting_pipeline,
            &composite_pipeline,
            &fallback_input,
            &light_buffer,
        );

        let (vertices, indices) = create_cube_mesh();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("deferred_rendering vertex_buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("deferred_rendering index_buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut example = Self {
            graph,
            active_subpass_mask,
            surface_size: [config.width, config.height],
            frame_index: 0,
            pass_attachments,
            fallback_input,
            scene,
            scene_buffer,
            light,
            light_buffer,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            geometry_bind_group,
            lighting_bind_group,
            composite_bind_group,
            geometry_pipeline,
            lighting_pipeline,
            composite_pipeline,
        };
        example.update_uniforms(queue);
        example
    }

    fn resize(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        self.surface_size = [config.width, config.height];
        self.recreate_runtime_targets(config, device);
        self.update_uniforms(queue);
    }

    fn update(&mut self, _event: winit::event::WindowEvent) {}

    fn render(&mut self, view: &wgpu::TextureView, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.update_uniforms(queue);

        let graph_views = self.graph.descriptor_views();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("deferred_rendering encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("deferred_rendering pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.pass_attachments.albedo_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.pass_attachments.normal_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.5,
                                g: 0.5,
                                b: 1.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.pass_attachments.lit_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.03,
                                g: 0.03,
                                b: 0.04,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.pass_attachments.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                subpasses: &graph_views.subpasses,
                subpass_dependencies: graph_views.subpass_dependencies,
                active_subpass_mask: Some(self.active_subpass_mask),
                transient_memory_hint: wgpu::TransientMemoryHint::Auto,
                multiview_mask: None,
                ..Default::default()
            });

            pass.set_pipeline(&self.geometry_pipeline);
            pass.set_bind_group(0, &self.geometry_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..INSTANCE_COUNT);

            pass.next_subpass();
            pass.set_pipeline(&self.lighting_pipeline);
            pass.set_bind_group(0, &self.lighting_bind_group, &[]);
            pass.draw(0..3, 0..1);

            pass.next_subpass();
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
        self.frame_index = self.frame_index.saturating_add(1);
    }
}

fn create_cube_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let vertices = vec![
        // +Z
        Vertex {
            position: [-1.0, -1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [1.0, -1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [1.0, 1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-1.0, 1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
        // -Z
        Vertex {
            position: [1.0, -1.0, -1.0],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [-1.0, -1.0, -1.0],
            normal: [0.0, 0.0, -1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [-1.0, 1.0, -1.0],
            normal: [0.0, 0.0, -1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [1.0, 1.0, -1.0],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 1.0],
        },
        // +X
        Vertex {
            position: [1.0, -1.0, 1.0],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [1.0, -1.0, -1.0],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [1.0, 1.0, -1.0],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [1.0, 1.0, 1.0],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 1.0],
        },
        // -X
        Vertex {
            position: [-1.0, -1.0, -1.0],
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [-1.0, -1.0, 1.0],
            normal: [-1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [-1.0, 1.0, 1.0],
            normal: [-1.0, 0.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-1.0, 1.0, -1.0],
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0, 1.0],
        },
        // +Y
        Vertex {
            position: [-1.0, 1.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [1.0, 1.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [1.0, 1.0, -1.0],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-1.0, 1.0, -1.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 1.0],
        },
        // -Y
        Vertex {
            position: [-1.0, -1.0, -1.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [1.0, -1.0, -1.0],
            normal: [0.0, -1.0, 0.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [1.0, -1.0, 1.0],
            normal: [0.0, -1.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-1.0, -1.0, 1.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.0, 1.0],
        },
    ];

    let indices = vec![
        0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4, 8, 9, 10, 10, 11, 8, 12, 13, 14, 14, 15, 12, 16, 17,
        18, 18, 19, 16, 20, 21, 22, 22, 23, 20,
    ];
    (vertices, indices)
}

pub fn main() {
    crate::framework::run::<Example>("deferred_rendering");
}

#[cfg(test)]
#[wgpu_test::gpu_test]
pub static TEST: crate::framework::ExampleTestParams = crate::framework::ExampleTestParams {
    name: "deferred-rendering",
    image_path: "/examples/features/src/deferred_rendering/screenshot.png",
    width: 1024,
    height: 768,
    optional_features: wgpu::Features::empty(),
    base_test_parameters: wgpu_test::TestParameters::default(),
    comparisons: &[wgpu_test::ComparisonType::Mean(0.02)],
    _phantom: std::marker::PhantomData::<Example>,
};
