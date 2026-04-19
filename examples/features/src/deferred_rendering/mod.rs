//! Deferred Rendering with 3-Subpass Render Graph
//!
//! Demonstrates a TBDR-optimized deferred rendering pipeline:
//!   Subpass 0 (G-Buffer): renders geometry to albedo + normal textures
//!   Subpass 1 (Lighting): reads G-Buffer via input attachments, Blinn-Phong with 4 lights → HDR
//!   Subpass 2 (Composite): reads HDR result, Reinhard tonemapping → sRGB swapchain
//!
//! Uses `RenderGraphBuilder` to declare the subpass graph with automatic
//! dependency inference and validation. On TBDR hardware (Metal, Vulkan mobile),
//! all three subpasses execute within a single hardware pass with G-Buffer data
//! staying in tile memory.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::borrow::Cow;
use wgpu::util::DeviceExt;

// Vertex with position, normal, color
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

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct LightParams {
    lights: [[f32; 4]; 4], // xyz = position, w = intensity
    camera_pos: [f32; 3],
    time: f32,
    inv_view_proj: [[f32; 4]; 4], // for world-position reconstruction from depth
    screen_size: [f32; 2],
    _padding: [f32; 2],
}

const GRID_SIZE: u32 = 5;
const INSTANCE_COUNT: u32 = GRID_SIZE * GRID_SIZE;

fn create_cube_vertices() -> (Vec<Vertex>, Vec<u16>) {
    let positions: &[[f32; 3]] = &[
        // Front
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
        // Back
        [-1.0, -1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [1.0, 1.0, -1.0],
        [1.0, -1.0, -1.0],
        // Top
        [-1.0, 1.0, -1.0],
        [-1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, -1.0],
        // Bottom
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, -1.0, 1.0],
        [-1.0, -1.0, 1.0],
        // Right
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [1.0, 1.0, 1.0],
        [1.0, -1.0, 1.0],
        // Left
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
        [1.0, 0.3, 0.3], // Front - red
        [0.3, 1.0, 0.3], // Back - green
        [0.3, 0.3, 1.0], // Top - blue
        [1.0, 1.0, 0.3], // Bottom - yellow
        [1.0, 0.3, 1.0], // Right - magenta
        [0.3, 1.0, 1.0], // Left - cyan
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

struct Example {
    gbuffer_pipeline: wgpu::RenderPipeline,
    lighting_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    uniform_buf: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    light_buf: wgpu::Buffer,
    lighting_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    albedo_texture: wgpu::Texture,
    normal_texture: wgpu::Texture,
    depth_texture: wgpu::Texture,
    lit_texture: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    normal_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    lit_view: wgpu::TextureView,
    start_time: std::time::Instant,
    width: u32,
    height: u32,
    render_graph: wgpu::RenderGraph,
}

fn create_gbuffer_textures(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::Texture, wgpu::Texture, wgpu::Texture) {
    // Transient textures: RENDER_ATTACHMENT | TRANSIENT enables MTLStorageModeMemoryless
    // on Metal. These live in tile memory only — no DRAM backing, no TEXTURE_BINDING.
    let transient_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TRANSIENT;

    let albedo = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("G-Buffer Albedo"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: transient_usage,
        view_formats: &[],
    });
    let normal = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("G-Buffer Normal"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: transient_usage,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: transient_usage,
        view_formats: &[],
    });
    let lit = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Lit HDR"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: transient_usage,
        view_formats: &[],
    });
    (albedo, normal, depth, lit)
}

impl crate::framework::Example for Example {
    fn required_features() -> wgpu::Features {
        wgpu::Features::MULTI_SUBPASS | wgpu::Features::TRANSIENT_ATTACHMENTS
    }

    fn required_limits() -> wgpu::Limits {
        wgpu::Limits {
            max_subpass_color_attachments: 2,
            max_subpasses: 3,
            ..wgpu::Limits::downlevel_webgl2_defaults()
        }
    }

    fn init(
        config: &wgpu::SurfaceConfiguration,
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        let width = config.width;
        let height = config.height;

        // -- Plan the render graph (3 subpasses) --
        //
        // Attachments:
        //   0 = albedo   (Rgba8Unorm)    — transient
        //   1 = normal   (Rgba16Float)   — transient
        //   2 = lit      (Rgba16Float)   — transient (HDR intermediate)
        //   3 = output   (swapchain fmt) — persistent
        //   + depth      (Depth32Float)  — transient
        //
        // Subpass 0 (G-Buffer):  writes color [0,1] + depth,  no inputs
        // Subpass 1 (Lighting):  reads [0,1],                 writes color [2]
        // Subpass 2 (Composite): reads [2],                   writes color [3]
        let mut graph_builder = wgpu::RenderGraphBuilder::new();
        let albedo_att =
            graph_builder.add_transient_color("albedo", wgpu::TextureFormat::Rgba8Unorm);
        let normal_att =
            graph_builder.add_transient_color("normal", wgpu::TextureFormat::Rgba16Float);
        let lit_att = graph_builder.add_transient_color("lit", wgpu::TextureFormat::Rgba16Float);
        let _depth_att =
            graph_builder.add_transient_depth("depth", wgpu::TextureFormat::Depth32Float);
        // Use the sRGB view format so the pipeline applies gamma correction consistently
        // across backends. The framework creates the swapchain view with view_formats[0]
        // (the sRGB variant), so the pipeline target must match.
        let output_format = config
            .view_formats
            .first()
            .copied()
            .unwrap_or(config.format);
        let output_att = graph_builder.add_persistent_color("output", output_format);

        graph_builder
            .add_subpass("gbuffer")
            .writes_color(albedo_att)
            .writes_color(normal_att);
        graph_builder
            .add_subpass("lighting")
            .reads(albedo_att)
            .reads(normal_att)
            .writes_color(lit_att);
        graph_builder
            .add_subpass("composite")
            .reads(lit_att)
            .writes_color(output_att);

        let render_graph = graph_builder.build().unwrap();
        log::info!(
            "Render graph: {} subpasses, {} dependencies",
            render_graph.subpass_count,
            render_graph.dependencies.len()
        );

        // -- Create textures --
        let (albedo_texture, normal_texture, depth_texture, lit_texture) =
            create_gbuffer_textures(device, width, height);

        let albedo_view = albedo_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let normal_view = normal_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let lit_view = lit_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // -- Create geometry (single cube, instanced 5x5 in shader) --
        let (vertices, indices) = create_cube_vertices();
        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // -- SubpassTarget: describes the full 3-subpass structure --
        //
        // Render pass color attachments (by index):
        //   0 = albedo   (Rgba8Unorm)
        //   1 = normal   (Rgba16Float)
        //   2 = lit      (Rgba16Float)
        //   3 = output   (swapchain format)
        //   + depth      (Depth32Float)
        let subpass_target_base = wgpu::SubpassTarget {
            index: 0,
            color_attachment_formats: vec![
                Some(wgpu::TextureFormat::Rgba8Unorm),
                Some(wgpu::TextureFormat::Rgba16Float),
                Some(wgpu::TextureFormat::Rgba16Float),
                Some(output_format),
            ],
            depth_stencil_format: Some(wgpu::TextureFormat::Depth32Float),
            subpass_descs: vec![
                wgpu::SubpassTargetDesc {
                    color_attachment_indices: vec![0, 1],
                    uses_depth_stencil: true,
                    input_attachment_indices: vec![],
                },
                wgpu::SubpassTargetDesc {
                    color_attachment_indices: vec![2],
                    uses_depth_stencil: false,
                    input_attachment_indices: vec![0, 1],
                },
                wgpu::SubpassTargetDesc {
                    color_attachment_indices: vec![3],
                    uses_depth_stencil: false,
                    input_attachment_indices: vec![2],
                },
            ],
            dependencies: render_graph.dependencies.clone(),
        };

        // -- G-Buffer pipeline (subpass 0) --
        let gbuffer_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("G-Buffer Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("gbuffer.wgsl"))),
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let gbuffer_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("G-Buffer BGL"),
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
            label: Some("Uniform BG"),
            layout: &gbuffer_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let gbuffer_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("G-Buffer Layout"),
            bind_group_layouts: &[Some(&gbuffer_bgl)],
            ..Default::default()
        });

        let subpass_target_sp0 = subpass_target_base.clone();
        let gbuffer_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("G-Buffer Pipeline"),
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
                targets: &[
                    Some(wgpu::TextureFormat::Rgba8Unorm.into()),
                    Some(wgpu::TextureFormat::Rgba16Float.into()),
                ],
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
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
            subpass_target: Some(subpass_target_sp0),
        });

        // -- Lighting pipeline (subpass 1) --
        let lighting_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lighting Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("lighting.wgsl"))),
        });

        let light_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Light Params"),
            size: std::mem::size_of::<LightParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fallback_input = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deferred Fallback Input"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let fallback_input_view =
            fallback_input.create_view(&wgpu::TextureViewDescriptor::default());
        queue.write_texture(
            fallback_input.as_image_copy(),
            &[0, 0, 0, 0, 0, 0, 0, 0],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );

        let subpass_target_sp1 = wgpu::SubpassTarget {
            index: 1,
            ..subpass_target_base.clone()
        };
        let lighting_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lighting Pipeline"),
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
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
            subpass_target: Some(subpass_target_sp1),
        });

        let lighting_bgl = lighting_pipeline.get_bind_group_layout(0);
        let lighting_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting BG"),
            layout: &lighting_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&fallback_input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&fallback_input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: light_buf.as_entire_binding(),
                },
            ],
        });

        // -- Composite pipeline (subpass 2) --
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("composite.wgsl"))),
        });

        let subpass_target_sp2 = wgpu::SubpassTarget {
            index: 2,
            ..subpass_target_base
        };
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Composite Pipeline"),
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
            subpass_target: Some(subpass_target_sp2),
        });
        let composite_bgl = composite_pipeline.get_bind_group_layout(0);
        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Composite BG"),
            layout: &composite_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&fallback_input_view),
            }],
        });

        let _ = queue; // suppress unused

        Self {
            gbuffer_pipeline,
            lighting_pipeline,
            composite_pipeline,
            vertex_buf,
            index_buf,
            index_count: indices.len() as u32,
            uniform_buf,
            uniform_bind_group,
            light_buf,
            lighting_bind_group,
            composite_bind_group,
            albedo_texture,
            normal_texture,
            depth_texture,
            lit_texture,
            albedo_view,
            normal_view,
            depth_view,
            lit_view,
            start_time: std::time::Instant::now(),
            width,
            height,
            render_graph,
        }
    }

    fn resize(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) {
        self.width = config.width;
        self.height = config.height;

        let (albedo, normal, depth, lit) = create_gbuffer_textures(device, self.width, self.height);
        self.albedo_texture = albedo;
        self.normal_texture = normal;
        self.depth_texture = depth;
        self.lit_texture = lit;

        self.albedo_view = self
            .albedo_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.normal_view = self
            .normal_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.depth_view = self
            .depth_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.lit_view = self
            .lit_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
    }

    fn update(&mut self, _event: winit::event::WindowEvent) {}

    fn render(&mut self, view: &wgpu::TextureView, device: &wgpu::Device, queue: &wgpu::Queue) {
        let time = self.start_time.elapsed().as_secs_f32();
        let aspect = self.width as f32 / self.height.max(1) as f32;

        // Camera orbiting the scene
        let eye = Vec3::new(
            12.0 * (time * 0.3).cos(),
            8.0,
            12.0 * (time * 0.3).sin() + 15.0,
        );
        let target = Vec3::ZERO;
        let view_mat = Mat4::look_at_rh(eye, target, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
        let view_proj = proj * view_mat;

        // Update uniforms
        queue.write_buffer(
            &self.uniform_buf,
            0,
            bytemuck::cast_slice(&[Uniforms {
                view_proj: view_proj.to_cols_array_2d(),
            }]),
        );

        // 4 orbiting point lights
        let t = time;
        let inv_view_proj = view_proj.inverse();
        queue.write_buffer(
            &self.light_buf,
            0,
            bytemuck::cast_slice(&[LightParams {
                lights: [
                    [10.0 * (t * 0.7).cos(), 8.0, 10.0 * (t * 0.7).sin(), 50.0],
                    [-8.0 * (t * 0.5).cos(), 6.0, -8.0 * (t * 0.5).sin(), 40.0],
                    [6.0 * (t * 1.1).sin(), 4.0, 6.0 * (t * 1.1).cos(), 35.0],
                    [-5.0, 10.0 + 3.0 * (t * 0.3).sin(), 5.0, 45.0],
                ],
                camera_pos: [eye.x, eye.y, eye.z],
                time: t,
                inv_view_proj: inv_view_proj.to_cols_array_2d(),
                screen_size: [self.width as f32, self.height as f32],
                _padding: [0.0; 2],
            }]),
        );

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Deferred Renderer"),
        });

        // === Single Render Pass with 3 Subpasses ===
        // NOTE: These SubpassDescriptors must stay in sync with the SubpassTargetDescs
        // used at pipeline creation time (subpass_target_base.subpass_descs above).
        let subpass_descs = [
            wgpu::SubpassDescriptor {
                color_attachment_indices: &[0, 1],
                uses_depth_stencil: true,
                input_attachments: &[],
            },
            wgpu::SubpassDescriptor {
                color_attachment_indices: &[2],
                uses_depth_stencil: false,
                // Bindings must match lighting.wgsl: @binding(0) = albedo, @binding(1) = normal
                input_attachments: &[
                    wgpu::SubpassInputAttachment {
                        binding: 0, // → @binding(0) @input_attachment_index(0) t_albedo
                        source: wgpu::SubpassInputSource::Color {
                            subpass: wgpu::SubpassIndex(0),
                            attachment_index: 0, // render pass color attachment 0
                        },
                    },
                    wgpu::SubpassInputAttachment {
                        binding: 1, // → @binding(1) @input_attachment_index(1) t_normal
                        source: wgpu::SubpassInputSource::Color {
                            subpass: wgpu::SubpassIndex(0),
                            attachment_index: 1, // render pass color attachment 1
                        },
                    },
                ],
            },
            wgpu::SubpassDescriptor {
                color_attachment_indices: &[3],
                uses_depth_stencil: false,
                // Binding must match composite.wgsl:
                // @binding(0) @input_attachment_index(2) = lit HDR color.
                input_attachments: &[wgpu::SubpassInputAttachment {
                    binding: 0, // → @binding(0) @input_attachment_index(2) t_lit_color
                    source: wgpu::SubpassInputSource::Color {
                        subpass: wgpu::SubpassIndex(1),
                        attachment_index: 0, // source subpass color slot 0 (lit = render-pass attachment 2)
                    },
                }],
            },
        ];

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Deferred (3 subpasses)"),
                color_attachments: &[
                    // Attachment 0: G-Buffer albedo
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.albedo_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    // Attachment 1: G-Buffer normal
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.normal_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    // Attachment 2: HDR lit result
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.lit_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    // Attachment 3: Swapchain output (composite writes here)
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                subpass_dependencies: &self.render_graph.dependencies,
                transient_memory_hint: wgpu::TransientMemoryHint::PreferTileMemory,
                subpasses: &subpass_descs,
                ..Default::default()
            });

            // Subpass 0: G-Buffer geometry (instanced 5x5 cube grid)
            pass.set_pipeline(&self.gbuffer_pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..INSTANCE_COUNT);

            // Subpass 1: Lighting (fullscreen, reads G-Buffer via input attachments)
            pass.next_subpass();
            pass.set_pipeline(&self.lighting_pipeline);
            pass.set_bind_group(0, &self.lighting_bind_group, &[]);
            pass.draw(0..3, 0..1);

            // Subpass 2: Composite (fullscreen, tonemapping → sRGB swapchain)
            pass.next_subpass();
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }
}

pub fn main() {
    crate::framework::run::<Example>("Deferred Rendering (3-Subpass Demo)");
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
