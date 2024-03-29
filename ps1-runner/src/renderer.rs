use anyhow::anyhow;
use ps1_core::api::{ColorDepthBits, RenderParams, Renderer};
use std::collections::HashMap;
use std::{cmp, iter};
use thiserror::Error;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    Backends, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    BufferBinding, BufferBindingType, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, CompositeAlphaMode, Device, DeviceDescriptor, Dx12Compiler, Extent3d,
    Features, FilterMode, FragmentState, FrontFace, Gles3MinorVersion, ImageCopyTexture,
    ImageDataLayout, Instance, InstanceDescriptor, InstanceFlags, Limits, LoadOp, MultisampleState,
    Operations, Origin3d, PipelineLayoutDescriptor, PolygonMode, PowerPreference, PresentMode,
    PrimitiveState, PrimitiveTopology, Queue, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipeline, RenderPipelineDescriptor, RequestAdapterOptions, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderModule, ShaderStages, StoreOp, Surface, SurfaceConfiguration,
    SurfaceError, SurfaceTargetUnsafe, Texture, TextureAspect, TextureDescriptor, TextureDimension,
    TextureFormat, TextureSampleType, TextureUsages, TextureViewDescriptor, TextureViewDimension,
    VertexState,
};
use winit::dpi::LogicalSize;
use winit::window::Window;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
struct Color {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl Color {
    const BLACK: Self = Self::rgb(0, 0, 0);

    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FrameSize {
    width: u32,
    height: u32,
}

enum FrameScaling {
    None { raw: Texture },
    Scaled { raw: Texture, scaled: Texture, bind_group: BindGroup, pipeline: RenderPipeline },
}

pub struct WgpuRenderer {
    surface: Surface<'static>,
    surface_config: SurfaceConfiguration,
    device: Device,
    queue: Queue,
    scale_module: ShaderModule,
    render_bind_group_layout: BindGroupLayout,
    render_pipeline: RenderPipeline,
    frame_buffer: Vec<Color>,
    frame_texture_format: TextureFormat,
    frame_textures: HashMap<FrameSize, FrameScaling>,
    sampler: Sampler,
    prescaling: bool,
    filter_mode: FilterMode,
    dumping_vram: bool,
    cropping_v_overscan: bool,
}

impl WgpuRenderer {
    /// # Safety
    ///
    /// The referenced window must live at least as long as the returned `WgpuRenderer`.
    pub async unsafe fn new(window: &Window) -> anyhow::Result<Self> {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            dx12_shader_compiler: Dx12Compiler::default(),
            gles_minor_version: Gles3MinorVersion::default(),
        });

        let surface_target = SurfaceTargetUnsafe::from_window(window)?;
        let surface = instance.create_surface_unsafe(surface_target)?;

        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await;
        let Some(adapter) = adapter else {
            return Err(anyhow!("Failed to obtain wgpu adapter"));
        };

        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    label: "device".into(),
                    required_features: Features::default(),
                    required_limits: Limits::default(),
                },
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
            width: window.inner_size().width,
            height: window.inner_size().height,
            present_mode: PresentMode::Mailbox,
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::default(),
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        let frame_texture_format = if surface_format.is_srgb() {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        };

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

        let scale_module = device.create_shader_module(wgpu::include_wgsl!("scale.wgsl"));

        Ok(Self {
            surface,
            surface_config,
            device,
            queue,
            scale_module,
            render_bind_group_layout,
            render_pipeline,
            frame_buffer: vec![Color::BLACK; 1024 * 512],
            frame_texture_format,
            frame_textures: HashMap::new(),
            sampler,
            prescaling: true,
            filter_mode,
            dumping_vram: false,
            cropping_v_overscan: true,
        })
    }

    pub fn handle_resize(&mut self, width: u32, height: u32) {
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn toggle_prescaling(&mut self) {
        self.prescaling = !self.prescaling;
        self.frame_textures.clear();

        log::info!("Prescaling enabled: {}", self.prescaling);
    }

    pub fn toggle_filter_mode(&mut self) {
        self.filter_mode = match self.filter_mode {
            FilterMode::Nearest => FilterMode::Linear,
            FilterMode::Linear => FilterMode::Nearest,
        };
        self.sampler = create_sampler(&self.device, self.filter_mode);

        log::info!("Current filter mode is {:?}", self.filter_mode);
    }

    pub fn toggle_dumping_vram(&mut self, window: &Window) {
        self.dumping_vram = !self.dumping_vram;

        if self.dumping_vram {
            let _ = window.request_inner_size(LogicalSize::new(1024, 512));
        } else {
            let _ = window.request_inner_size(LogicalSize::new(585, 448));
        }

        log::info!("Dumping VRAM: {}", self.dumping_vram);
    }

    pub fn toggle_cropping_v_overscan(&mut self) {
        self.cropping_v_overscan = !self.cropping_v_overscan;

        log::info!("Cropping vertical overscan: {}", self.cropping_v_overscan);
    }

    // TODO pay attention to display width and X offset?
    fn populate_frame_buffer(&mut self, vram: &[u8], params: RenderParams) {
        log::debug!("Populating frame buffer using parameters {params:?}");

        if self.dumping_vram {
            for (i, chunk) in vram.chunks_exact(2).enumerate() {
                let rgb555_color = u16::from_le_bytes([chunk[0], chunk[1]]);
                self.frame_buffer[i] = convert_rgb555_color(rgb555_color);
            }
            return;
        }

        if !params.display_enabled || params.display_width == 0 || params.display_height == 0 {
            self.frame_buffer[..(params.frame_width * params.frame_height) as usize]
                .fill(Color::BLACK);
            return;
        }

        if params.display_y_offset > 0 {
            self.frame_buffer[..(params.frame_width * params.display_y_offset as u32) as usize]
                .fill(Color::BLACK);
        }

        let effective_display_height = if params.frame_height == 480 {
            params.display_height * 2
        } else {
            params.display_height
        };

        let y_begin = cmp::max(0, params.display_y_offset) as u32;
        let y_end = cmp::min(
            params.frame_height,
            effective_display_height.wrapping_add_signed(params.display_y_offset),
        );
        for y in y_begin..y_end {
            let vram_y =
                y.wrapping_add(params.frame_y).wrapping_add_signed(-params.display_y_offset) & 511;
            let vram_row = (2048 * vram_y) as usize;
            let frame_buffer_row = (params.frame_width * y) as usize;

            // Always treat frame X as a halfword coordinate, even in 24bpp mode
            // Final Fantasy 8 depends on this for FMVs
            let vram_row_start = 2 * params.frame_x;

            match params.color_depth {
                ColorDepthBits::Fifteen => {
                    for x in 0..params.frame_width {
                        let vram_row_offset = (vram_row_start + 2 * x) & 2047;
                        let vram_addr = vram_row + vram_row_offset as usize;

                        let rgb555_color =
                            u16::from_le_bytes([vram[vram_addr], vram[vram_addr + 1]]);
                        self.frame_buffer[frame_buffer_row + x as usize] =
                            convert_rgb555_color(rgb555_color);
                    }
                }
                ColorDepthBits::TwentyFour => {
                    for x in 0..params.frame_width {
                        let vram_row_offset = (vram_row_start + 3 * x) as usize;

                        let r = vram[vram_row | (vram_row_offset & 2047)];
                        let g = vram[vram_row | ((vram_row_offset + 1) & 2047)];
                        let b = vram[vram_row | ((vram_row_offset + 2) & 2047)];
                        self.frame_buffer[frame_buffer_row + x as usize] = Color::rgb(r, g, b);
                    }
                }
            }
        }

        if y_end < params.frame_height {
            self.frame_buffer[(params.frame_width * y_end) as usize
                ..(params.frame_width * params.frame_height) as usize]
                .fill(Color::BLACK);
        }
    }
}

fn convert_rgb555_color(rgb555_color: u16) -> Color {
    let r = RGB_5_TO_8[(rgb555_color & 0x1F) as usize];
    let g = RGB_5_TO_8[((rgb555_color >> 5) & 0x1F) as usize];
    let b = RGB_5_TO_8[((rgb555_color >> 10) & 0x1F) as usize];

    Color::rgb(r, g, b)
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

fn create_frame_scaling(
    device: &Device,
    scale_module: &ShaderModule,
    format: TextureFormat,
    size: FrameSize,
    prescaling: bool,
) -> FrameScaling {
    log::info!("Creating {}x{} frame texture", size.width, size.height);

    let raw = device.create_texture(&TextureDescriptor {
        label: format!("raw_frame_texture_{}x{}", size.width, size.height).as_str().into(),
        size: Extent3d { width: size.width, height: size.height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });

    if !prescaling || (size.width >= 512 && size.height >= 448) {
        return FrameScaling::None { raw };
    }

    let scaled_width = if size.width < 512 { 2 * size.width } else { size.width };
    let scaled_height = if size.height < 448 { 2 * size.height } else { size.height };
    log::info!(
        "Scaling {}x{} frame to {scaled_width}x{scaled_height} before display",
        size.width,
        size.height
    );

    let scaled = device.create_texture(&TextureDescriptor {
        label: format!("scaled_frame_texture_{scaled_width}x{scaled_height}").as_str().into(),
        size: Extent3d { width: scaled_width, height: scaled_height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: "scale_bind_group_layout".into(),
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
        contents: &(scaled_width / size.width).to_le_bytes(),
        usage: BufferUsages::UNIFORM,
    });
    let height_scale_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: "height_scale_buffer".into(),
        contents: &(scaled_height / size.height).to_le_bytes(),
        usage: BufferUsages::UNIFORM,
    });

    let raw_view = raw.create_view(&TextureViewDescriptor::default());

    let bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: "scale_bind_group".into(),
        layout: &bind_group_layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&raw_view) },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &width_scale_buffer,
                    offset: 0,
                    size: None,
                }),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &height_scale_buffer,
                    offset: 0,
                    size: None,
                }),
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: "scale_pipeline_layout".into(),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: "scale_pipeline".into(),
        layout: Some(&pipeline_layout),
        vertex: VertexState { module: scale_module, entry_point: "vs_main", buffers: &[] },
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
            module: scale_module,
            entry_point: "fs_main",
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(BlendState::REPLACE),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview: None,
    });

    FrameScaling::Scaled { raw, scaled, bind_group, pipeline }
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

fn create_render_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    texture: &Texture,
    sampler: &Sampler,
) -> BindGroup {
    let texture_view = texture.create_view(&TextureViewDescriptor::default());

    device.create_bind_group(&BindGroupDescriptor {
        label: "render_bind_group".into(),
        layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&texture_view) },
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
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: "render_pipeline".into(),
        layout: Some(&pipeline_layout),
        vertex: VertexState { module: shader_module, entry_point: "vs_main", buffers: &[] },
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
            targets: &[Some(ColorTargetState {
                format: output_format,
                blend: Some(BlendState::REPLACE),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview: None,
    })
}

#[derive(Debug, Error)]
pub enum WgpuError {
    #[error("wgpu surface error: {0}")]
    Surface(#[from] SurfaceError),
}

const RGB_5_TO_8: &[u8; 32] = &[
    0, 8, 16, 25, 33, 41, 49, 58, 66, 74, 82, 90, 99, 107, 115, 123, 132, 140, 148, 156, 165, 173,
    181, 189, 197, 206, 214, 222, 230, 239, 247, 255,
];

impl Renderer for WgpuRenderer {
    type Err = WgpuError;

    fn render_frame(&mut self, vram: &[u8], params: RenderParams) -> Result<(), Self::Err> {
        self.populate_frame_buffer(vram, params);

        let (overscan_rows_top, frame_size) = if self.dumping_vram {
            (0, FrameSize { width: 1024, height: 512 })
        } else if self.cropping_v_overscan {
            let cropped_frame_height = params.frame_height * 14 / 15;
            let overscan_rows_top = params.frame_height / 30;

            let frame_size = FrameSize { width: params.frame_width, height: cropped_frame_height };

            (overscan_rows_top, frame_size)
        } else {
            (0, FrameSize { width: params.frame_width, height: params.frame_height })
        };

        let frame_scaling = self.frame_textures.entry(frame_size).or_insert_with(|| {
            create_frame_scaling(
                &self.device,
                &self.scale_module,
                self.frame_texture_format,
                frame_size,
                self.prescaling,
            )
        });

        let raw_texture = match frame_scaling {
            FrameScaling::None { raw } | FrameScaling::Scaled { raw, .. } => raw,
        };

        self.queue.write_texture(
            ImageCopyTexture {
                texture: raw_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            bytemuck::cast_slice(&self.frame_buffer),
            ImageDataLayout {
                offset: (frame_size.width * 4 * overscan_rows_top).into(),
                bytes_per_row: Some(frame_size.width * 4),
                rows_per_image: None,
            },
            Extent3d {
                width: frame_size.width,
                height: frame_size.height,
                depth_or_array_layers: 1,
            },
        );

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: "command_encoder".into() });

        if let FrameScaling::Scaled { scaled, bind_group, pipeline, .. } = frame_scaling {
            let scaled_view = scaled.create_view(&TextureViewDescriptor::default());

            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "scale_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &scaled_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.set_pipeline(pipeline);

            render_pass.draw(0..4, 0..1);
        }

        let output_texture = self.surface.get_current_texture()?;
        let output_view = output_texture.texture.create_view(&TextureViewDescriptor::default());

        let frame_texture = match frame_scaling {
            FrameScaling::None { raw: texture } | FrameScaling::Scaled { scaled: texture, .. } => {
                texture
            }
        };

        let render_bind_group = create_render_bind_group(
            &self.device,
            &self.render_bind_group_layout,
            frame_texture,
            &self.sampler,
        );

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &output_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_bind_group(0, &render_bind_group, &[]);
            render_pass.set_pipeline(&self.render_pipeline);

            render_pass.draw(0..4, 0..1);
        }

        self.queue.submit(iter::once(encoder.finish()));
        output_texture.present();

        Ok(())
    }
}
