use crate::config::VideoConfig;
use crate::emuthread::{EmulatorSwapChain, TextureWithAspectRatio};
use crate::{emuthread, Never};
use ps1_core::api::Renderer;
use std::collections::{HashMap, VecDeque};
use std::iter;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use winit::dpi::PhysicalSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FrameSize {
    width: u32,
    height: u32,
}

impl From<&wgpu::Texture> for FrameSize {
    fn from(value: &wgpu::Texture) -> Self {
        Self { width: value.width(), height: value.height() }
    }
}

pub struct SwapChainRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    swap_chain: EmulatorSwapChain,
    swap_chain_textures: HashMap<FrameSize, VecDeque<wgpu::Texture>>,
    in_progress_renders: Arc<AtomicU32>,
    texture_buffer: VecDeque<wgpu::Texture>,
}

impl SwapChainRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        swap_chain: EmulatorSwapChain,
    ) -> Self {
        Self {
            device,
            queue,
            swap_chain,
            swap_chain_textures: HashMap::new(),
            in_progress_renders: Arc::new(AtomicU32::new(0)),
            texture_buffer: VecDeque::with_capacity(emuthread::SWAP_CHAIN_LEN),
        }
    }

    fn reclaim_returned_textures(&mut self) {
        self.texture_buffer.extend(self.swap_chain.returned_frames.lock().unwrap().drain(..));

        while let Some(texture) = self.texture_buffer.pop_front() {
            let frame_size = FrameSize::from(&texture);
            let entry = self.swap_chain_textures.entry(frame_size).or_default();
            entry.push_back(texture);
            while entry.len() > emuthread::SWAP_CHAIN_LEN {
                entry.pop_front();
            }
        }
    }

    pub fn clear_swap_chain(&self) {
        self.swap_chain.rendered_frames.lock().unwrap().clear();
    }
}

impl Renderer for SwapChainRenderer {
    type Err = Never;

    fn render_frame(
        &mut self,
        command_buffers: impl Iterator<Item = wgpu::CommandBuffer>,
        frame: &wgpu::Texture,
        pixel_aspect_ratio: f64,
    ) -> Result<(), Self::Err> {
        self.reclaim_returned_textures();

        // Load/compare followed by increment is fine because this value is only incremented from one
        // thread; other thread modifications are decrements
        if self.in_progress_renders.load(Ordering::Relaxed) >= emuthread::SWAP_CHAIN_LEN as u32 {
            log::warn!(
                "Skipping frame because {} renders are already in progress",
                emuthread::SWAP_CHAIN_LEN
            );
            self.queue.submit(command_buffers);
            return Ok(());
        }

        self.in_progress_renders.fetch_add(1, Ordering::Relaxed);

        let frame_size = FrameSize::from(frame);
        let swap_chain_texture =
            self.swap_chain_textures.entry(frame_size).or_default().pop_front().unwrap_or_else(
                || {
                    log::debug!("Creating new swap chain texture of size {frame_size:?}");

                    self.device.create_texture(&wgpu::TextureDescriptor {
                        label: "swap_chain_texture".into(),
                        size: frame.size(),
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: frame.dimension(),
                        format: frame.format(),
                        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                    })
                },
            );

        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            swap_chain_texture.as_image_copy(),
            frame.size(),
        );
        let submission = self.queue.submit(command_buffers.chain(iter::once(encoder.finish())));

        let push_texture = TextureWithAspectRatio(swap_chain_texture, pixel_aspect_ratio);

        if self.swap_chain.async_rendering.load(Ordering::Relaxed) {
            let in_progress_renders = Arc::clone(&self.in_progress_renders);
            let rendered_frames = Arc::clone(&self.swap_chain.rendered_frames);
            let returned_frames = Arc::clone(&self.swap_chain.returned_frames);
            self.queue.on_submitted_work_done(move || {
                let popped_texture = rendered_frames.lock().unwrap().push_back(push_texture);
                if let Some(popped_texture) = popped_texture {
                    returned_frames.lock().unwrap().push_back(popped_texture);
                }

                in_progress_renders.fetch_sub(1, Ordering::Relaxed);
            });
        } else {
            self.device.poll(wgpu::Maintain::WaitForSubmissionIndex(submission));

            let popped_texture =
                self.swap_chain.rendered_frames.lock().unwrap().push_back(push_texture);
            if let Some(popped_texture) = popped_texture {
                let frame_size = FrameSize::from(&popped_texture);
                self.swap_chain_textures.entry(frame_size).or_default().push_back(popped_texture);
            }

            self.in_progress_renders.fetch_sub(1, Ordering::Relaxed);
        }

        Ok(())
    }
}

pub struct SurfaceRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    swap_chain: EmulatorSwapChain,
    surface_size: wgpu::Extent3d,
    sampler_bind_group_layout: wgpu::BindGroupLayout,
    sampler_bind_group: wgpu::BindGroup,
    frame_bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl SurfaceRenderer {
    pub fn new(
        config: &VideoConfig,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        swap_chain: EmulatorSwapChain,
        surface_config: &wgpu::SurfaceConfiguration,
    ) -> Self {
        let sampler_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: "sampler_bind_group_layout".into(),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                }],
            });

        let filter_mode = config.filter_mode.to_wgpu();
        let sampler_bind_group =
            create_sampler_bind_group(&device, &sampler_bind_group_layout, filter_mode);

        let frame_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: "frame_bind_group_layout".into(),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: "render_pipeline_layout".into(),
            bind_group_layouts: &[&sampler_bind_group_layout, &frame_bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("render.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: "render_pipeline".into(),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: "vs_main", buffers: &[] },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });

        Self {
            device,
            queue,
            swap_chain,
            surface_size: wgpu::Extent3d {
                width: surface_config.width,
                height: surface_config.height,
                depth_or_array_layers: 1,
            },
            sampler_bind_group_layout,
            sampler_bind_group,
            frame_bind_group_layout,
            pipeline,
        }
    }

    pub fn update_config(&mut self, config: &VideoConfig) {
        self.sampler_bind_group = create_sampler_bind_group(
            &self.device,
            &self.sampler_bind_group_layout,
            config.filter_mode.to_wgpu(),
        );
    }

    pub fn handle_resize(&mut self, size: PhysicalSize<u32>) {
        self.surface_size.width = size.width;
        self.surface_size.height = size.height;
    }

    pub fn render_frame_if_available(&mut self, surface: &wgpu::Surface<'_>) -> anyhow::Result<()> {
        let Some(TextureWithAspectRatio(frame, pixel_aspect_ratio)) =
            self.swap_chain.rendered_frames.lock().unwrap().pop_front()
        else {
            return Ok(());
        };

        let output = surface.get_current_texture()?;
        let output_view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let frame_view = frame.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            ..wgpu::TextureViewDescriptor::default()
        });
        let frame_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: "frame_bind_group".into(),
            layout: &self.frame_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&frame_view),
            }],
        });

        let viewport = determine_viewport(frame.size(), self.surface_size, pixel_aspect_ratio);
        log::trace!("Rendering to viewport {viewport:?}");

        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: "surface_render_pass".into(),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..wgpu::RenderPassDescriptor::default()
            });

            render_pass.set_viewport(
                viewport.x,
                viewport.y,
                viewport.width,
                viewport.height,
                0.0,
                1.0,
            );

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.sampler_bind_group, &[]);
            render_pass.set_bind_group(1, &frame_bind_group, &[]);

            render_pass.draw(0..4, 0..1);
        }

        self.queue.submit(iter::once(encoder.finish()));
        output.present();

        self.swap_chain.returned_frames.lock().unwrap().push_back(frame);

        Ok(())
    }
}

fn create_sampler_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    filter_mode: wgpu::FilterMode,
) -> wgpu::BindGroup {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: "frame_sampler".into(),
        mag_filter: filter_mode,
        min_filter: filter_mode,
        mipmap_filter: filter_mode,
        ..wgpu::SamplerDescriptor::default()
    });

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: "sampler_bind_group".into(),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Sampler(&sampler),
        }],
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
    frame_size: wgpu::Extent3d,
    surface_size: wgpu::Extent3d,
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
