use anyhow::anyhow;
use ps1_core::api::Renderer;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{cmp, iter, thread};
use wgpu::rwh::{HasDisplayHandle, HasWindowHandle};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    Backends, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBinding, BufferBindingType, BufferUsages, Color, ColorTargetState, ColorWrites,
    CommandBuffer, CommandEncoder, CommandEncoderDescriptor, CompositeAlphaMode, Device,
    DeviceDescriptor, Extent3d, Features, FilterMode, FragmentState, FrontFace, Instance,
    InstanceDescriptor, Limits, LoadOp, MultisampleState, Operations, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PolygonMode, PowerPreference, PresentMode, PrimitiveState,
    PrimitiveTopology, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, RequestAdapterOptions, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderModule, ShaderStages, StoreOp, Surface, SurfaceConfiguration,
    SurfaceTargetUnsafe, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureViewDescriptor, TextureViewDimension, VertexState,
};

struct PrescalePipeline {
    texture: Texture,
    bind_group_layout: BindGroupLayout,
    width_scale_buffer: Buffer,
    height_scale_buffer: Buffer,
    pipeline: RenderPipeline,
}

impl PrescalePipeline {
    fn new(
        device: &Device,
        scale_shader: &ShaderModule,
        frame_size: Extent3d,
        window_size: Extent3d,
        pixel_aspect_ratio: f64,
    ) -> Self {
        let viewport = determine_viewport(frame_size, window_size, pixel_aspect_ratio);

        let width_scale = cmp::max(
            1,
            (viewport.width / frame_size.width as f32 / pixel_aspect_ratio as f32 + 0.01).floor()
                as u32,
        );
        let height_scale =
            cmp::max(1, (viewport.height / frame_size.height as f32 + 0.01).floor() as u32);

        let texture = device.create_texture(&TextureDescriptor {
            label: "scaled_texture".into(),
            size: Extent3d {
                width: width_scale * frame_size.width,
                height: height_scale * frame_size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        log::info!(
            "Scaling frame of size {}x{} to {}x{}",
            frame_size.width,
            frame_size.height,
            texture.size().width,
            texture.size().height
        );

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "prescale_bind_group_layout".into(),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: false },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let width_scale_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: "width_scale_buffer".into(),
            contents: &width_scale.to_le_bytes(),
            usage: BufferUsages::UNIFORM,
        });
        let height_scale_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: "height_scale_buffer".into(),
            contents: &height_scale.to_le_bytes(),
            usage: BufferUsages::UNIFORM,
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "prescale_pipeline_layout".into(),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: "prescale_pipeline".into(),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: scale_shader,
                entry_point: "vs_main",
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: scale_shader,
                entry_point: "fs_main",
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8UnormSrgb,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });

        Self { texture, bind_group_layout, width_scale_buffer, height_scale_buffer, pipeline }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PrescaleKey {
    frame_width: u32,
    frame_height: u32,
    pixel_aspect_ratio_bits: u64,
}

impl PrescaleKey {
    fn new(frame_size: Extent3d, pixel_aspect_ratio: f64) -> Self {
        Self {
            frame_width: frame_size.width,
            frame_height: frame_size.height,
            pixel_aspect_ratio_bits: pixel_aspect_ratio.to_bits(),
        }
    }
}

pub struct WgpuRenderer {
    surface: Surface<'static>,
    surface_config: SurfaceConfiguration,
    device: Rc<Device>,
    queue: Rc<Queue>,
    render_bind_group_layout: BindGroupLayout,
    render_pipeline: RenderPipeline,
    sampler: Sampler,
    filter_mode: FilterMode,
    auto_prescale: bool,
    prescale_shader: ShaderModule,
    prescale_pipelines: HashMap<PrescaleKey, PrescalePipeline>,
}

impl WgpuRenderer {
    /// # Safety
    ///
    /// The value referenced by `window` must live at least as long as the returned `WgpuRenderer`.
    pub async unsafe fn new<W>(
        window: &W,
        window_size: (u32, u32),
        present_mode: PresentMode,
        required_features: Features,
        required_limits: Limits,
    ) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle,
    {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            ..InstanceDescriptor::default()
        });

        let surface = instance.create_surface_unsafe(SurfaceTargetUnsafe::from_window(window)?)?;

        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .ok_or_else(|| anyhow!("Failed to obtain wgpu adapter"))?;

        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor { label: "device".into(), required_features, required_limits },
                None,
            )
            .await?;

        let surface_formats = &surface.get_capabilities(&adapter).formats;
        let surface_format =
            surface_formats.iter().copied().find(TextureFormat::is_srgb).unwrap_or_else(|| {
                log::warn!(
                    "Surface does not support any SRGB formats, using format {:?}",
                    surface_formats[0]
                );
                surface_formats[0]
            });

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: window_size.0,
            height: window_size.1,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::default(),
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        let render_bind_group_layout = create_render_bind_group_layout(&device);

        let shader_module = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let render_pipeline = create_render_pipeline(
            &device,
            &shader_module,
            &render_bind_group_layout,
            surface_format,
        );

        let filter_mode = FilterMode::Linear;
        let sampler = create_sampler(&device, filter_mode);

        let prescale_shader = device.create_shader_module(wgpu::include_wgsl!("scale.wgsl"));

        Ok(Self {
            surface,
            surface_config,
            device: Rc::new(device),
            queue: Rc::new(queue),
            render_bind_group_layout,
            render_pipeline,
            sampler,
            filter_mode,
            auto_prescale: true,
            prescale_shader,
            prescale_pipelines: HashMap::new(),
        })
    }

    pub fn handle_resize(&mut self, width: u32, height: u32) {
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);

        self.prescale_pipelines.clear();
    }

    pub fn toggle_filter_mode(&mut self) {
        self.filter_mode = match self.filter_mode {
            FilterMode::Linear => FilterMode::Nearest,
            FilterMode::Nearest => FilterMode::Linear,
        };
        self.sampler = create_sampler(&self.device, self.filter_mode);

        log::info!("Filter mode is now {:?}", self.filter_mode);
    }

    pub fn toggle_prescaling(&mut self) {
        self.auto_prescale = !self.auto_prescale;
        self.prescale_pipelines.clear();

        log::info!("Auto prescaling on: {}", self.auto_prescale);
    }

    pub fn device(&self) -> Rc<Device> {
        Rc::clone(&self.device)
    }

    pub fn queue(&self) -> Rc<Queue> {
        Rc::clone(&self.queue)
    }

    pub fn block_until_done(&self) -> anyhow::Result<()> {
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        self.queue.on_submitted_work_done(move || done_clone.store(true, Ordering::Relaxed));

        // Just in case there is not any active GPU work
        self.queue.submit(iter::empty());

        let start = Instant::now();
        while !done.load(Ordering::Relaxed)
            && Instant::now().duration_since(start) < Duration::from_secs(1)
        {
            thread::sleep(Duration::from_millis(10));
        }

        if !done.load(Ordering::Relaxed) {
            return Err(anyhow!("Timed out waiting for wgpu queue work to finish"));
        }

        Ok(())
    }
}

impl Renderer for WgpuRenderer {
    type Err = anyhow::Error;

    fn render_frame(
        &mut self,
        command_buffers: impl Iterator<Item = CommandBuffer>,
        frame: &Texture,
        pixel_aspect_ratio: f64,
    ) -> Result<(), Self::Err> {
        let output = self.surface.get_current_texture()?;
        let output_view = output.texture.create_view(&TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: "render_encoder".into() });

        let input_texture = if self.auto_prescale {
            let prescale_pipeline = self
                .prescale_pipelines
                .entry(PrescaleKey::new(frame.size(), pixel_aspect_ratio))
                .or_insert_with(|| {
                    PrescalePipeline::new(
                        &self.device,
                        &self.prescale_shader,
                        frame.size(),
                        output.texture.size(),
                        pixel_aspect_ratio,
                    )
                });

            execute_prescale_pipeline(&self.device, prescale_pipeline, frame, &mut encoder);

            &prescale_pipeline.texture
        } else {
            frame
        };

        let render_bind_group = create_render_bind_group(
            &self.device,
            &self.render_bind_group_layout,
            input_texture,
            &self.sampler,
        );

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "output_render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &output_view,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Clear(Color::BLACK), store: StoreOp::Store },
                })],
                ..RenderPassDescriptor::default()
            });

            let viewport =
                determine_viewport(frame.size(), output.texture.size(), pixel_aspect_ratio);
            render_pass.set_viewport(
                viewport.x,
                viewport.y,
                viewport.width,
                viewport.height,
                0.0,
                1.0,
            );

            render_pass.set_bind_group(0, &render_bind_group, &[]);
            render_pass.set_pipeline(&self.render_pipeline);

            render_pass.draw(0..4, 0..1);
        }

        self.queue.submit(command_buffers.chain(iter::once(encoder.finish())));
        output.present();

        Ok(())
    }
}

fn create_render_bind_group_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: "render_bind_group_layout".into(),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn create_render_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    frame: &Texture,
    sampler: &Sampler,
) -> BindGroup {
    let frame_view = frame.create_view(&TextureViewDescriptor {
        format: Some(TextureFormat::Rgba8UnormSrgb),
        ..TextureViewDescriptor::default()
    });

    device.create_bind_group(&BindGroupDescriptor {
        label: "render_bind_group".into(),
        layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&frame_view) },
            BindGroupEntry { binding: 1, resource: BindingResource::Sampler(sampler) },
        ],
    })
}

fn create_render_pipeline(
    device: &Device,
    shader_module: &ShaderModule,
    bind_group_layout: &BindGroupLayout,
    output_format: TextureFormat,
) -> RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: "render_pipeline_layout".into(),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: "render_pipeline".into(),
        layout: Some(&pipeline_layout),
        vertex: VertexState {
            module: shader_module,
            entry_point: "vs_main",
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
        fragment: Some(FragmentState {
            module: shader_module,
            entry_point: "fs_main",
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format: output_format,
                blend: Some(BlendState::REPLACE),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview: None,
    })
}

fn create_sampler(device: &Device, filter_mode: FilterMode) -> Sampler {
    device.create_sampler(&SamplerDescriptor {
        label: "sampler".into(),
        mag_filter: filter_mode,
        min_filter: filter_mode,
        mipmap_filter: filter_mode,
        ..SamplerDescriptor::default()
    })
}

#[derive(Debug)]
struct Viewport {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn determine_viewport(
    frame_size: Extent3d,
    surface_size: Extent3d,
    pixel_aspect_ratio: f64,
) -> Viewport {
    let aspect_correct_width =
        (f64::from(surface_size.height) * pixel_aspect_ratio * f64::from(frame_size.width)
            / f64::from(frame_size.height)) as f32;
    if aspect_correct_width <= surface_size.width as f32 {
        return Viewport {
            x: (surface_size.width as f32 - aspect_correct_width) / 2.0,
            y: 0.0,
            width: aspect_correct_width,
            height: surface_size.height as f32,
        };
    }

    let aspect_correct_height = (f64::from(surface_size.width) / pixel_aspect_ratio
        * f64::from(frame_size.height)
        / f64::from(frame_size.width)) as f32;
    Viewport {
        x: 0.0,
        y: (surface_size.height as f32 - aspect_correct_height) / 2.0,
        width: surface_size.width as f32,
        height: aspect_correct_height,
    }
}

fn execute_prescale_pipeline(
    device: &Device,
    pipeline: &PrescalePipeline,
    frame: &Texture,
    encoder: &mut CommandEncoder,
) {
    let frame_view = frame.create_view(&TextureViewDescriptor {
        format: Some(TextureFormat::Rgba8UnormSrgb),
        ..TextureViewDescriptor::default()
    });

    let bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: "prescale_bind_group".into(),
        layout: &pipeline.bind_group_layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&frame_view) },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &pipeline.width_scale_buffer,
                    offset: 0,
                    size: None,
                }),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &pipeline.height_scale_buffer,
                    offset: 0,
                    size: None,
                }),
            },
        ],
    });

    let scaled_view = pipeline.texture.create_view(&TextureViewDescriptor::default());

    let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: "prescale_render_pass".into(),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &scaled_view,
            resolve_target: None,
            ops: Operations { load: LoadOp::Clear(Color::BLACK), store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });

    render_pass.set_bind_group(0, &bind_group, &[]);
    render_pass.set_pipeline(&pipeline.pipeline);

    render_pass.draw(0..4, 0..1);
}
