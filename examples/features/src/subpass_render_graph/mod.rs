async fn run() {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .expect("No suitable adapter found");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("subpass_render_graph device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })
        .await
        .expect("Failed to create device");

    if !device.features().contains(wgpu::Features::MULTI_SUBPASS) {
        log::warn!("Adapter does not support MULTI_SUBPASS; skipping headless subpass graph pass");
        return;
    }

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let extent = wgpu::Extent3d {
        width: 4,
        height: 4,
        depth_or_array_layers: 1,
    };
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    let transient_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("subpass_render_graph transient"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("subpass_render_graph output"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let transient_view = transient_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut graph_builder = wgpu::RenderGraphBuilder::new();
    graph_builder.sample_count(1);
    let gbuffer = graph_builder.add_transient_color("gbuffer", format);
    let output = graph_builder.add_persistent_color("output", format);
    let _ = graph_builder.add_subpass("gbuffer").writes_color(gbuffer);
    let _ = graph_builder
        .add_subpass("composite")
        .reads(gbuffer)
        .writes_color(output);
    let graph = graph_builder.build().expect("Render graph build failed");
    let views = graph.descriptor_views();
    let active_mask = graph
        .resolve_active(&[wgpu::SubpassIndex(0), wgpu::SubpassIndex(1)])
        .expect("Failed to resolve active subpasses");

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("subpass_render_graph encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("subpass_render_graph pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &transient_view,
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
        pass.next_subpass();
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
