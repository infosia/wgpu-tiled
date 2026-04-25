//! Minimal MSAA subpass-input example.
//!
//! Subpass 0 writes MSAA albedo + depth.
//! Subpass 1 reads multisampled input attachments with `@builtin(sample_index)` and writes HDR.
//! A follow-up pass resolves/tonemaps to the swapchain.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::borrow::Cow;
use wgpu::util::DeviceExt;

const GRID_SIZE: u32 = 5;
const INSTANCE_COUNT: u32 = GRID_SIZE * GRID_SIZE;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

fn create_cube_vertices() -> (Vec<Vertex>, Vec<u16>) {
    let positions: &[[f32; 3]] = &[
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
        [-1.0, -1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [1.0, 1.0, -1.0],
        [1.0, -1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, -1.0],
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, -1.0, 1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [1.0, 1.0, 1.0],
        [1.0, -1.0, 1.0],
        [-1.0, -1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [-1.0, 1.0, 1.0],
        [-1.0, 1.0, -1.0],
    ];
    let normals: &[[f32; 3]] = &[
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
    ];
    let face_colors: &[[f32; 3]] = &[
        [1.0, 0.4, 0.3],
        [0.3, 1.0, 0.4],
        [0.3, 0.5, 1.0],
        [1.0, 1.0, 0.3],
        [1.0, 0.3, 1.0],
        [0.3, 1.0, 1.0],
    ];

    let vertices: Vec<Vertex> = (0..24)
        .map(|i| Vertex {
            position: positions[i],
            normal: normals[i],
            color: face_colors[i / 4],
        })
        .collect();

    let indices: Vec<u16> = (0..6u16)
        .flat_map(|face| {
            let base = face * 4;
            [base, base + 1, base + 2, base, base + 2, base + 3]
        })
        .collect();

    (vertices, indices)
}

fn write_uniforms(queue: &wgpu::Queue, uniform_buf: &wgpu::Buffer, width: u32, height: u32) {
    let aspect = width as f32 / height.max(1) as f32;
    let eye = Vec3::new(14.0, 10.0, 14.0);
    let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
    let view_proj = proj * view;
    queue.write_buffer(
        uniform_buf,
        0,
        bytemuck::cast_slice(&[Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
        }]),
    );
}

fn choose_msaa_sample_count(adapter: &wgpu::Adapter) -> Option<u32> {
    let mut common = wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2
        | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
        | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X8
        | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X16;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Depth32Float,
    ] {
        common &= adapter.get_texture_format_features(format).flags;
    }
    if common.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4) {
        Some(4)
    } else if common.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2) {
        Some(2)
    } else {
        None
    }
}

struct FrameTextures {
    _albedo: wgpu::Texture,
    _lit_ms: wgpu::Texture,
    _depth: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    lit_ms_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

impl FrameTextures {
    fn new(device: &wgpu::Device, width: u32, height: u32, sample_count: u32) -> Self {
        let transient_usage =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TRANSIENT;
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let albedo = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("subpass_msaa albedo"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: transient_usage,
            view_formats: &[],
        });
        let lit_ms = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("subpass_msaa lit_ms"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("subpass_msaa depth"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: transient_usage,
            view_formats: &[],
        });
        let albedo_view = albedo.create_view(&wgpu::TextureViewDescriptor::default());
        let lit_ms_view = lit_ms.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            _albedo: albedo,
            _lit_ms: lit_ms,
            _depth: depth,
            albedo_view,
            lit_ms_view,
            depth_view,
        }
    }
}

struct Renderer {
    sample_count: u32,
    gbuffer_pipeline: wgpu::RenderPipeline,
    lighting_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    uniform_buf: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    lighting_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    composite_bgl: wgpu::BindGroupLayout,
    frame: FrameTextures,
    render_graph: wgpu::RenderGraph,
}

struct Example {
    renderer: Renderer,
}

impl crate::framework::Example for Example {
    fn required_features() -> wgpu::Features {
        wgpu::Features::MULTI_SUBPASS | wgpu::Features::TRANSIENT_ATTACHMENTS
    }

    fn required_limits() -> wgpu::Limits {
        wgpu::Limits {
            max_input_attachments: 1,
            max_subpass_color_attachments: 1,
            max_subpasses: 2,
            ..wgpu::Limits::downlevel_webgl2_defaults()
        }
    }

    fn required_downlevel_capabilities() -> wgpu::DownlevelCapabilities {
        wgpu::DownlevelCapabilities {
            flags: wgpu::DownlevelFlags::MULTISAMPLED_SHADING,
            shader_model: wgpu::ShaderModel::Sm5,
            ..wgpu::DownlevelCapabilities::default()
        }
    }

    fn init(
        config: &wgpu::SurfaceConfiguration,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        let backend = adapter.get_info().backend;
        assert!(
            matches!(backend, wgpu::Backend::Metal | wgpu::Backend::Vulkan),
            "subpass_msaa supports only Metal/Vulkan backends"
        );
        let sample_count = choose_msaa_sample_count(adapter)
            .expect("subpass_msaa requires at least 2x MSAA support for Rgba8Unorm, Rgba16Float, and Depth32Float");
        if sample_count != 4 {
            log::warn!("subpass_msaa using {sample_count}x MSAA (4x unavailable on this adapter)");
        }

        let output_format = config
            .view_formats
            .first()
            .copied()
            .unwrap_or(config.format);

        let mut graph_builder = wgpu::RenderGraphBuilder::new();
        graph_builder.sample_count(sample_count);
        let albedo_att =
            graph_builder.add_transient_color("albedo_ms", wgpu::TextureFormat::Rgba8Unorm);
        let lit_att = graph_builder.add_transient_color("lit_ms", wgpu::TextureFormat::Rgba16Float);
        let depth_att =
            graph_builder.add_transient_depth("depth_ms", wgpu::TextureFormat::Depth32Float);

        graph_builder
            .add_subpass("gbuffer")
            .writes_color(albedo_att)
            .writes_depth(depth_att);
        graph_builder
            .add_subpass("lighting")
            .reads(albedo_att)
            .writes_color(lit_att);

        let render_graph = graph_builder.build().unwrap();
        let color_attachment_formats = [
            Some(wgpu::TextureFormat::Rgba8Unorm),
            Some(wgpu::TextureFormat::Rgba16Float),
        ];
        let depth_stencil_format = Some(wgpu::TextureFormat::Depth32Float);

        let frame = FrameTextures::new(device, config.width, config.height, sample_count);

        let (vertices, indices) = create_cube_vertices();
        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("subpass_msaa vertex"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("subpass_msaa index"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("subpass_msaa uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        write_uniforms(queue, &uniform_buf, config.width, config.height);

        let gbuffer_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("subpass_msaa gbuffer"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("gbuffer.wgsl"))),
        });
        let gbuffer_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("subpass_msaa gbuffer_bgl"),
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
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("subpass_msaa uniforms_bg"),
            layout: &gbuffer_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let gbuffer_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("subpass_msaa gbuffer_layout"),
            bind_group_layouts: &[Some(&gbuffer_bgl)],
            ..Default::default()
        });
        let gbuffer_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("subpass_msaa gbuffer_pipeline"),
            layout: Some(&gbuffer_layout),
            vertex: wgpu::VertexState {
                module: &gbuffer_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &gbuffer_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
            subpass_target: Some(render_graph.make_subpass_target(
                0,
                &color_attachment_formats,
                depth_stencil_format,
            )),
        });

        let lighting_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("subpass_msaa lighting"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("lighting.wgsl"))),
        });
        let lighting_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("subpass_msaa lighting_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &lighting_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &lighting_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
            subpass_target: Some(render_graph.make_subpass_target(
                1,
                &color_attachment_formats,
                depth_stencil_format,
            )),
        });
        let lighting_bgl = lighting_pipeline.get_bind_group_layout(0);
        let fallback_albedo_ms = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("subpass_msaa fallback_albedo_ms"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let fallback_albedo_ms_view =
            fallback_albedo_ms.create_view(&wgpu::TextureViewDescriptor::default());
        let lighting_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("subpass_msaa lighting_bg"),
            layout: &lighting_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&fallback_albedo_ms_view),
            }],
        });

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("subpass_msaa composite"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("composite.wgsl"))),
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("subpass_msaa composite_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(output_format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
            subpass_target: None,
        });
        let composite_bgl = composite_pipeline.get_bind_group_layout(0);
        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("subpass_msaa composite_bg"),
            layout: &composite_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&frame.lit_ms_view),
            }],
        });

        Self {
            renderer: Renderer {
                sample_count,
                gbuffer_pipeline,
                lighting_pipeline,
                composite_pipeline,
                vertex_buf,
                index_buf,
                index_count: indices.len() as u32,
                uniform_buf,
                uniform_bind_group,
                lighting_bind_group,
                composite_bind_group,
                composite_bgl,
                frame,
                render_graph,
            },
        }
    }

    fn resize(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let renderer = &mut self.renderer;
        renderer.frame =
            FrameTextures::new(device, config.width, config.height, renderer.sample_count);
        write_uniforms(queue, &renderer.uniform_buf, config.width, config.height);
        renderer.composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("subpass_msaa composite_bg"),
            layout: &renderer.composite_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&renderer.frame.lit_ms_view),
            }],
        });
    }

    fn update(&mut self, _event: winit::event::WindowEvent) {}

    fn render(&mut self, view: &wgpu::TextureView, device: &wgpu::Device, queue: &wgpu::Queue) {
        let renderer = &mut self.renderer;

        let graph_views = renderer.render_graph.descriptor_views();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("subpass_msaa encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subpass_msaa graph"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &renderer.frame.albedo_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &renderer.frame.lit_ms_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.frame.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                subpasses: &graph_views.subpasses,
                subpass_dependencies: graph_views.subpass_dependencies,
                transient_memory_hint: wgpu::TransientMemoryHint::PreferTileMemory,
                ..Default::default()
            });

            pass.set_pipeline(&renderer.gbuffer_pipeline);
            pass.set_bind_group(0, &renderer.uniform_bind_group, &[]);
            pass.set_vertex_buffer(0, renderer.vertex_buf.slice(..));
            pass.set_index_buffer(renderer.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..renderer.index_count, 0, 0..INSTANCE_COUNT);

            pass.next_subpass();
            pass.set_pipeline(&renderer.lighting_pipeline);
            pass.set_bind_group(0, &renderer.lighting_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subpass_msaa composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&renderer.composite_pipeline);
            pass.set_bind_group(0, &renderer.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }
}

pub fn main() {
    crate::framework::run::<Example>("MSAA Subpass Inputs");
}
