use anyhow::anyhow;
use ps1_core::api::Renderer;
use std::iter;
use thiserror::Error;
use wgpu::{
    Backends, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    ColorTargetState, ColorWrites, CommandEncoderDescriptor, CompositeAlphaMode, Device,
    DeviceDescriptor, Dx12Compiler, Extent3d, Features, FilterMode, FragmentState, FrontFace,
    Gles3MinorVersion, ImageCopyTexture, ImageDataLayout, Instance, InstanceDescriptor,
    InstanceFlags, Limits, LoadOp, MultisampleState, Operations, Origin3d,
    PipelineLayoutDescriptor, PolygonMode, PowerPreference, PresentMode, PrimitiveState,
    PrimitiveTopology, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, RequestAdapterOptions, SamplerBindingType, SamplerDescriptor,
    ShaderStages, StoreOp, Surface, SurfaceConfiguration, SurfaceError, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureViewDescriptor, TextureViewDimension, VertexState,
};
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

pub struct WgpuRenderer<'window> {
    surface: Surface<'window>,
    surface_config: SurfaceConfiguration,
    device: Device,
    queue: Queue,
    render_bind_group: BindGroup,
    render_pipeline: RenderPipeline,
    frame_buffer: Vec<Color>,
    frame_texture: Texture,
}

impl<'window> WgpuRenderer<'window> {
    pub async fn new(window: &'window Window) -> anyhow::Result<Self> {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            dx12_shader_compiler: Dx12Compiler::default(),
            gles_minor_version: Gles3MinorVersion::default(),
        });

        let surface = instance.create_surface(window)?;

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

        let frame_texture = device.create_texture(&TextureDescriptor {
            label: "frame_texture".into(),
            size: Extent3d { width: 1024, height: 512, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: if surface_format.is_srgb() {
                TextureFormat::Rgba8UnormSrgb
            } else {
                TextureFormat::Rgba8Unorm
            },
            usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let (render_bind_group_layout, render_bind_group) =
            create_render_bind_group(&device, &frame_texture);

        let render_pipeline =
            create_render_pipeline(&device, &render_bind_group_layout, surface_format);

        Ok(Self {
            surface,
            surface_config,
            device,
            queue,
            render_bind_group,
            render_pipeline,
            frame_buffer: vec![Color::BLACK; 1024 * 512],
            frame_texture,
        })
    }

    pub fn handle_resize(&mut self, width: u32, height: u32) {
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }
}

fn create_render_bind_group(
    device: &Device,
    frame_texture: &Texture,
) -> (BindGroupLayout, BindGroup) {
    let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
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
    });

    let texture_view = frame_texture.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: "sampler".into(),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: FilterMode::Linear,
        ..SamplerDescriptor::default()
    });

    let bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: "render_bind_group".into(),
        layout: &bind_group_layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&texture_view) },
            BindGroupEntry { binding: 1, resource: BindingResource::Sampler(&sampler) },
        ],
    });

    (bind_group_layout, bind_group)
}

fn create_render_pipeline(
    device: &Device,
    bind_group_layout: &BindGroupLayout,
    output_format: TextureFormat,
) -> RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: "render_pipeline_layout".into(),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let shader_module = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: "render_pipeline".into(),
        layout: Some(&pipeline_layout),
        vertex: VertexState { module: &shader_module, entry_point: "vs_main", buffers: &[] },
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
            module: &shader_module,
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

impl<'window> Renderer for WgpuRenderer<'window> {
    type Err = WgpuError;

    fn render_frame(&mut self, vram: &[u8]) -> Result<(), Self::Err> {
        for (i, chunk) in vram.chunks_exact(2).enumerate() {
            let rgb555_color = u16::from_le_bytes([chunk[0], chunk[1]]);
            let r = RGB_5_TO_8[(rgb555_color & 0x1F) as usize];
            let g = RGB_5_TO_8[((rgb555_color >> 5) & 0x1F) as usize];
            let b = RGB_5_TO_8[((rgb555_color >> 10) & 0x1F) as usize];

            self.frame_buffer[i] = Color::rgb(r, g, b);
        }

        self.queue.write_texture(
            ImageCopyTexture {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            bytemuck::cast_slice(&self.frame_buffer),
            ImageDataLayout { offset: 0, bytes_per_row: Some(1024 * 4), rows_per_image: Some(512) },
            Extent3d { width: 1024, height: 512, depth_or_array_layers: 1 },
        );

        let output_texture = self.surface.get_current_texture()?;

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: "command_encoder".into() });

        let output_view = output_texture.texture.create_view(&TextureViewDescriptor::default());

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

            render_pass.set_bind_group(0, &self.render_bind_group, &[]);
            render_pass.set_pipeline(&self.render_pipeline);

            render_pass.draw(0..4, 0..1);
        }

        self.queue.submit(iter::once(encoder.finish()));
        output_texture.present();

        Ok(())
    }
}
