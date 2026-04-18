use wgpu_test::{
    gpu_test, image::ReadbackBuffers, GpuTestConfiguration, GpuTestInitializer, TestParameters,
    TestingContext,
};

pub fn all_tests(vec: &mut Vec<GpuTestInitializer>) {
    vec.push(DEFERRED_SUBPASS_SMOKE);
}

#[gpu_test]
static DEFERRED_SUBPASS_SMOKE: GpuTestConfiguration = GpuTestConfiguration::new()
    .parameters(
        TestParameters::default()
            .features(wgpu::Features::MULTI_SUBPASS | wgpu::Features::TRANSIENT_ATTACHMENTS)
            .limits(wgpu::Limits {
                max_subpasses: 2,
                max_subpass_color_attachments: 2,
                max_input_attachments: 1,
                ..wgpu::Limits::downlevel_defaults()
            }),
    )
    .run_async(run_test);

async fn run_test(ctx: TestingContext) {
    let adapter_backend = ctx.adapter_info.backend;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let extent = wgpu::Extent3d {
        width: 1,
        height: 1,
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

    let gbuffer_texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deferred gbuffer"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let output_texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deferred output"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let gbuffer_view = gbuffer_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let fallback_input_texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deferred fallback_input"),
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
    // Keep a real sampled input only on GLES Tier B. Other paths source input attachments
    // directly from render-pass state and only need this binding slot for validation.
    let needs_sampled_input_attachment = adapter_backend == wgpu::Backend::Gl
        && !ctx
            .device
            .features()
            .contains(wgpu::Features::FRAMEBUFFER_FETCH);
    let input_binding_view = if needs_sampled_input_attachment {
        &gbuffer_view
    } else {
        &fallback_input_view
    };

    let gbuffer_shader = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deferred gbuffer shader"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f(3.0, -1.0),
        vec2f(-1.0, 3.0)
    );
    return vec4f(positions[index], 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4f {
    return vec4f(64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0);
}
"#
                .into(),
            ),
        });
    let lighting_shader = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deferred lighting shader"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) @input_attachment_index(0)
var gbuffer_color: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f(3.0, -1.0),
        vec2f(-1.0, 3.0)
    );
    return vec4f(positions[index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let coord = vec2<i32>(i32(position.x), i32(position.y));
    return textureLoad(gbuffer_color, coord);
}
"#
                .into(),
            ),
        });
    let color_attachment_formats = [Some(format), Some(format)];
    let gbuffer_target = graph.make_subpass_target(0, &color_attachment_formats, None);
    let lighting_target = graph.make_subpass_target(1, &color_attachment_formats, None);

    let gbuffer_pipeline = ctx
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("deferred gbuffer pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &gbuffer_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &gbuffer_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
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
    let lighting_pipeline = ctx
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("deferred lighting pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &lighting_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &lighting_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
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
    let lighting_bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("deferred lighting bg"),
        layout: &lighting_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(input_binding_view),
        }],
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("deferred pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &gbuffer_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
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
            ..Default::default()
        });

        pass.set_pipeline(&gbuffer_pipeline);
        pass.draw(0..3, 0..1);
        pass.next_subpass();
        pass.set_bind_group(0, &lighting_bg, &[]);
        pass.set_pipeline(&lighting_pipeline);
        pass.draw(0..3, 0..1);
    }

    let readback = ReadbackBuffers::new(&ctx.device, &output_texture);
    readback.copy_from(&ctx.device, &mut encoder, &output_texture);

    ctx.queue.submit([encoder.finish()]);

    readback
        .assert_buffer_contents(&ctx, &[64, 128, 192, 255])
        .await;
}
