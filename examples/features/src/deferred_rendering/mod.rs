async fn run() {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .expect("No suitable adapter found");
    let adapter_backend = adapter.get_info().backend;
    let required_features = wgpu::Features::MULTI_SUBPASS | wgpu::Features::TRANSIENT_ATTACHMENTS;
    if !adapter.features().contains(required_features) {
        log::warn!("Adapter does not support {required_features:?}; skipping deferred_rendering");
        return;
    }

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("deferred_rendering device"),
            required_features,
            required_limits: wgpu::Limits {
                max_subpasses: 2,
                max_subpass_color_attachments: 2,
                max_input_attachments: 1,
                ..wgpu::Limits::downlevel_defaults()
            },
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })
        .await
        .expect("Failed to create device");

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let extent = wgpu::Extent3d {
        width: 4,
        height: 4,
        depth_or_array_layers: 1,
    };

    let mut graph_builder = wgpu::RenderGraphBuilder::new();
    graph_builder.sample_count(1);
    let gbuffer = graph_builder.add_transient_color("gbuffer", format);
    let output = graph_builder.add_persistent_color("output", format);
    let _ = graph_builder.add_subpass("gbuffer").writes_color(gbuffer);
    let _ = graph_builder
        .add_subpass("lighting")
        .reads(gbuffer)
        .writes_color(output);
    let graph = graph_builder.build().expect("Render graph build failed");
    let views = graph.descriptor_views();

    let active_mask = graph
        .resolve_active(&[wgpu::SubpassIndex(0), wgpu::SubpassIndex(1)])
        .expect("Failed to resolve active subpasses");

    let attachment_usage =
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let gbuffer_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deferred_rendering gbuffer"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: attachment_usage,
        view_formats: &[],
    });
    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deferred_rendering output"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: attachment_usage,
        view_formats: &[],
    });
    let gbuffer_view = gbuffer_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let fallback_input_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deferred_rendering fallback_input"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let fallback_input_view =
        fallback_input_texture.create_view(&wgpu::TextureViewDescriptor::default());
    // The WGSL binding is required for validation, but non-GLES-Tier-B backends source input
    // attachments from render-pass state (Vulkan descriptor remap, Metal color input, or GLES
    // Tier A framebuffer-fetch emission). Bind a benign placeholder view for those paths.
    let needs_sampled_input_attachment = adapter_backend == wgpu::Backend::Gl
        && !device
            .features()
            .contains(wgpu::Features::FRAMEBUFFER_FETCH);
    let input_binding_view = if needs_sampled_input_attachment {
        &gbuffer_view
    } else {
        &fallback_input_view
    };

    let gbuffer_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("deferred_rendering gbuffer shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.15, 0.45, 0.9, 1.0);
}
"#
            .into(),
        ),
    });
    let lighting_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("deferred_rendering lighting shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) @input_attachment_index(0)
var gbuffer_color: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(i32(position.x), i32(position.y));
    let albedo = textureLoad(gbuffer_color, coord);
    let lighting = vec3<f32>(1.0, 0.8, 0.6);
    return vec4<f32>(albedo.rgb * lighting, 1.0);
}
"#
            .into(),
        ),
    });

    let color_attachment_formats = [Some(format), Some(format)];
    let gbuffer_target = graph.make_subpass_target(0, &color_attachment_formats, None);
    let lighting_target = graph.make_subpass_target(1, &color_attachment_formats, None);

    let gbuffer_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("deferred_rendering gbuffer pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &gbuffer_shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &gbuffer_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        subpass_target: Some(gbuffer_target),
        cache: None,
    });
    let lighting_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("deferred_rendering lighting pipeline"),
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
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        subpass_target: Some(lighting_target),
        cache: None,
    });
    let lighting_bgl = lighting_pipeline.get_bind_group_layout(0);
    let lighting_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("deferred_rendering lighting bg"),
        layout: &lighting_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(input_binding_view),
        }],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("deferred_rendering encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("deferred_rendering pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &gbuffer_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            subpasses: &views.subpasses,
            subpass_dependencies: views.subpass_dependencies,
            active_subpass_mask: Some(active_mask),
            transient_memory_hint: wgpu::TransientMemoryHint::Auto,
            multiview_mask: None,
            ..Default::default()
        });
        pass.set_pipeline(&gbuffer_pipeline);
        pass.draw(0..3, 0..1);
        pass.next_subpass();
        pass.set_bind_group(0, &lighting_bg, &[]);
        pass.set_pipeline(&lighting_pipeline);
        pass.draw(0..3, 0..1);
    }
    queue.submit(Some(encoder.finish()));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

pub fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .format_timestamp_nanos()
            .init();
        pollster::block_on(run());
    }
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).expect("could not initialize logger");
        crate::utils::add_web_nothing_to_see_msg();
        wasm_bindgen_futures::spawn_local(run());
    }
}
