//! MSAA line demo routed through typed subpass inputs.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use winit::{
    event::{ElementState, KeyEvent, WindowEvent},
    keyboard::{Key, NamedKey},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    _pos: [f32; 2],
    _color: [f32; 4],
}

struct ActivePipelines {
    render_graph: wgpu::RenderGraph,
    gbuffer_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,
    present_bind_group: wgpu::BindGroup,
    resolve_pipeline: Option<wgpu::RenderPipeline>,
    resolve_bind_group: Option<wgpu::BindGroup>,
    _lines_texture: wgpu::Texture,
    lines_view: wgpu::TextureView,
    _output_texture: Option<wgpu::Texture>,
    output_view: Option<wgpu::TextureView>,
    _fallback_texture: wgpu::Texture,
}

struct Example {
    config: wgpu::SurfaceConfiguration,
    gbuffer_shader: wgpu::ShaderModule,
    present_shader: wgpu::ShaderModule,
    resolve_shader: wgpu::ShaderModule,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    sample_count: u32,
    max_sample_count: u32,
    rebuild: bool,
    active: ActivePipelines,
}

impl Example {
    fn max_sample_count(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> u32 {
        let sample_flags = adapter.get_texture_format_features(format).flags;
        if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X16) {
            16
        } else if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X8) {
            8
        } else if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4) {
            4
        } else if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2) {
            2
        } else {
            1
        }
    }

    fn create_line_vertices() -> Vec<Vertex> {
        let mut vertex_data = Vec::new();
        let max = 50;
        for i in 0..max {
            let percent = i as f32 / max as f32;
            let (sin, cos) = (percent * 2.0 * std::f32::consts::PI).sin_cos();
            vertex_data.push(Vertex {
                _pos: [0.0, 0.0],
                _color: [1.0, -sin, cos, 1.0],
            });
            vertex_data.push(Vertex {
                _pos: [cos, sin],
                _color: [sin, -cos, 1.0, 1.0],
            });
        }
        vertex_data
    }

    fn create_active_pipelines(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        sample_count: u32,
        gbuffer_shader: &wgpu::ShaderModule,
        present_shader: &wgpu::ShaderModule,
        resolve_shader: &wgpu::ShaderModule,
    ) -> ActivePipelines {
        let format = config.view_formats[0];

        let mut graph_builder = wgpu::RenderGraphBuilder::new();
        graph_builder.sample_count(sample_count);
        let lines_attachment = graph_builder.add_transient_color("lines_color", format);
        let output_attachment = graph_builder.add_transient_color("output", format);
        graph_builder
            .add_subpass("lines")
            .writes_color(lines_attachment);
        graph_builder
            .add_subpass("present")
            .reads(lines_attachment)
            .writes_color(output_attachment);
        let render_graph = graph_builder.build().unwrap();

        let color_attachment_formats = [Some(format), Some(format)];

        let gbuffer_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("subpass_msaa gbuffer_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: gbuffer_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: gbuffer_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
            subpass_target: Some(render_graph.make_subpass_target(
                0,
                &color_attachment_formats,
                None,
            )),
        });

        let present_entry_point = if sample_count == 1 {
            "fs_main_1x"
        } else {
            "fs_main_msaa"
        };
        let present_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("subpass_msaa present_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: present_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: present_shader,
                entry_point: Some(present_entry_point),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
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
                None,
            )),
        });

        let fallback_usage = if sample_count == 1 {
            wgpu::TextureUsages::TEXTURE_BINDING
        } else {
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT
        };
        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("subpass_msaa fallback_input"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: fallback_usage,
            view_formats: &[],
        });
        let fallback_view = fallback_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let present_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("subpass_msaa present_bind_group"),
            layout: &present_pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&fallback_view),
            }],
        });

        let extent = wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        let lines_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("subpass_msaa lines_color"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TRANSIENT,
            view_formats: &[],
        });
        let lines_view = lines_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let (output_texture, output_view, resolve_pipeline, resolve_bind_group) =
            if sample_count == 1 {
                (None, None, None, None)
            } else {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("subpass_msaa output_msaa"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("subpass_msaa resolve_pipeline"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: resolve_shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: resolve_shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(format.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                    subpass_target: None,
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("subpass_msaa resolve_bind_group"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    }],
                });

                (Some(texture), Some(view), Some(pipeline), Some(bind_group))
            };

        ActivePipelines {
            render_graph,
            gbuffer_pipeline,
            present_pipeline,
            present_bind_group,
            resolve_pipeline,
            resolve_bind_group,
            _lines_texture: lines_texture,
            lines_view,
            _output_texture: output_texture,
            output_view,
            _fallback_texture: fallback_texture,
        }
    }
}

impl crate::framework::Example for Example {
    fn optional_features() -> wgpu::Features {
        wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
    }

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
        _queue: &wgpu::Queue,
    ) -> Self {
        let backend = adapter.get_info().backend;
        assert!(
            matches!(backend, wgpu::Backend::Metal | wgpu::Backend::Vulkan),
            "subpass_msaa supports only Metal/Vulkan backends"
        );

        log::info!("Press left/right arrow keys to change sample_count.");

        let max_sample_count = Self::max_sample_count(adapter, config.view_formats[0]);
        let sample_count = max_sample_count;

        let gbuffer_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("subpass_msaa gbuffer_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "gbuffer.wgsl"
            ))),
        });
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("subpass_msaa present_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "present.wgsl"
            ))),
        });
        let resolve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("subpass_msaa resolve_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "resolve.wgsl"
            ))),
        });

        let vertex_data = Self::create_line_vertices();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("subpass_msaa vertex_buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let vertex_count = vertex_data.len() as u32;

        let active = Self::create_active_pipelines(
            device,
            config,
            sample_count,
            &gbuffer_shader,
            &present_shader,
            &resolve_shader,
        );

        Self {
            config: config.clone(),
            gbuffer_shader,
            present_shader,
            resolve_shader,
            vertex_buffer,
            vertex_count,
            sample_count,
            max_sample_count,
            rebuild: false,
            active,
        }
    }

    fn update(&mut self, event: WindowEvent) {
        if let WindowEvent::KeyboardInput {
            event:
                KeyEvent {
                    logical_key,
                    state: ElementState::Pressed,
                    ..
                },
            ..
        } = event
        {
            match logical_key {
                Key::Named(NamedKey::ArrowLeft) => {
                    if self.sample_count == self.max_sample_count {
                        self.sample_count = 1;
                        self.rebuild = true;
                    }
                }
                Key::Named(NamedKey::ArrowRight) => {
                    if self.sample_count == 1 {
                        self.sample_count = self.max_sample_count;
                        self.rebuild = true;
                    }
                }
                _ => {}
            }
        }
    }

    fn resize(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) {
        self.config = config.clone();
        self.rebuild = true;
    }

    fn render(&mut self, view: &wgpu::TextureView, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.rebuild {
            self.active = Self::create_active_pipelines(
                device,
                &self.config,
                self.sample_count,
                &self.gbuffer_shader,
                &self.present_shader,
                &self.resolve_shader,
            );
            self.rebuild = false;
        }

        let graph_views = self.active.render_graph.descriptor_views();
        let output_attachment = if self.sample_count == 1 {
            wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            }
        } else {
            wgpu::RenderPassColorAttachment {
                view: self.active.output_view.as_ref().unwrap(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            }
        };

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subpass_msaa pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.active.lines_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    }),
                    Some(output_attachment),
                ],
                depth_stencil_attachment: None,
                subpasses: &graph_views.subpasses,
                subpass_dependencies: graph_views.subpass_dependencies,
                transient_memory_hint: wgpu::TransientMemoryHint::PreferTileMemory,
                ..Default::default()
            });

            pass.set_pipeline(&self.active.gbuffer_pipeline);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);

            pass.next_subpass();
            pass.set_pipeline(&self.active.present_pipeline);
            pass.set_bind_group(0, &self.active.present_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if self.sample_count > 1 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subpass_msaa resolve"),
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
            pass.set_pipeline(self.active.resolve_pipeline.as_ref().unwrap());
            pass.set_bind_group(0, self.active.resolve_bind_group.as_ref().unwrap(), &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }
}

pub fn main() {
    crate::framework::run::<Example>("subpass-msaa");
}

#[cfg(test)]
#[wgpu_test::gpu_test]
pub static TEST: crate::framework::ExampleTestParams = crate::framework::ExampleTestParams {
    name: "subpass-msaa",
    image_path: "/examples/features/src/subpass_msaa/screenshot.png",
    width: 1024,
    height: 768,
    optional_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
    base_test_parameters: wgpu_test::TestParameters::default(),
    comparisons: &[
        wgpu_test::ComparisonType::Mean(0.065),
        wgpu_test::ComparisonType::Percentile {
            percentile: 0.5,
            threshold: 0.29,
        },
    ],
    _phantom: std::marker::PhantomData::<Example>,
};
