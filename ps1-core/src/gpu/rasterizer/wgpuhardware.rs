use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::{
    ClearPipeline, CpuVramBlitArgs, DrawLineArgs, DrawRectangleArgs, DrawTriangleArgs, FrameSize,
    RasterizerInterface, VramVramBlitArgs,
};
use crate::gpu::registers::Registers;
use crate::gpu::{rasterizer, Color, Vram, WgpuResources};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::mem;
use std::rc::Rc;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferBinding,
    BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandBuffer, CommandEncoder, CommandEncoderDescriptor, ComputePassDescriptor,
    ComputePipeline, ComputePipelineDescriptor, Device, Extent3d, FragmentState, FrontFace,
    ImageCopyTexture, LoadOp, MultisampleState, Operations, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology, PushConstantRange,
    Queue, RenderPass, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, ShaderModule, ShaderStages, StorageTextureAccess, StoreOp, Texture,
    TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureViewDescriptor, TextureViewDimension, VertexAttribute, VertexBufferLayout, VertexState,
    VertexStepMode,
};

const VRAM_WIDTH: u32 = 1024;
const VRAM_HEIGHT: u32 = 512;

// Must match CpuVramBlitArgs in cpuvramblit.wgsl
#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct ShaderCpuVramBlitArgs {
    position: [u32; 2],
    size: [u32; 2],
    force_mask_bit: u32,
    check_mask_bit: u32,
}

#[derive(Debug)]
struct CpuVramBlitPipeline {
    ram_buffer: Vec<u32>,
    bind_group_0: BindGroup,
    bind_group_layout_1: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl CpuVramBlitPipeline {
    // Must match X/Y workgroup size in shader
    const WORKGROUP_SIZE: u32 = 16;

    fn new(device: &Device, native_vram: &Texture) -> Self {
        let buffer = device.create_buffer(&BufferDescriptor {
            label: "cpu_vram_blit_buffer".into(),
            size: u64::from(VRAM_WIDTH * VRAM_HEIGHT) * mem::size_of::<u32>() as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let bind_group_layout_0 = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "cpu_vram_blit_bind_group_layout_0".into(),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::StorageTexture {
                    access: StorageTextureAccess::ReadWrite,
                    format: native_vram.format(),
                    view_dimension: TextureViewDimension::D2,
                },
                count: None,
            }],
        });

        let bind_group_layout_1 = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "cpu_vram_blit_bind_group_layout_1".into(),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let native_vram_view = native_vram.create_view(&TextureViewDescriptor::default());
        let bind_group_0 = device.create_bind_group(&BindGroupDescriptor {
            label: "cpu_vram_blit_bind_group".into(),
            layout: &bind_group_layout_0,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&native_vram_view),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "cpu_vram_blit_pipeline_layout".into(),
            bind_group_layouts: &[&bind_group_layout_0, &bind_group_layout_1],
            push_constant_ranges: &[PushConstantRange {
                stages: ShaderStages::COMPUTE,
                range: 0..mem::size_of::<ShaderCpuVramBlitArgs>() as u32,
            }],
        });

        let shader =
            device.create_shader_module(wgpu::include_wgsl!("wgpuhardware/cpuvramblit.wgsl"));
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: "cpu_vram_blit_pipeline".into(),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "cpu_vram_blit",
            compilation_options: PipelineCompilationOptions::default(),
        });

        Self {
            ram_buffer: Vec::with_capacity((VRAM_WIDTH * VRAM_HEIGHT) as usize),
            bind_group_0,
            bind_group_layout_1,
            pipeline,
        }
    }

    fn prepare(&mut self, device: &Device, args: &CpuVramBlitArgs, buffer: &[u16]) -> BindGroup {
        let copy_len = (args.width * args.height) as usize;

        self.ram_buffer.clear();
        self.ram_buffer.extend(buffer.iter().copied().map(u32::from).take(copy_len));

        let buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: "cpu_vram_blit_buffer".into(),
            contents: bytemuck::cast_slice(&self.ram_buffer),
            usage: BufferUsages::STORAGE,
        });

        device.create_bind_group(&BindGroupDescriptor {
            label: "cpu_vram_blit_bind_group_1".into(),
            layout: &self.bind_group_layout_1,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        })
    }

    fn dispatch(
        &self,
        args: CpuVramBlitArgs,
        bind_group_1: BindGroup,
        encoder: &mut CommandEncoder,
    ) {
        let shader_args = ShaderCpuVramBlitArgs {
            position: [args.x, args.y],
            size: [args.width, args.height],
            force_mask_bit: args.force_mask_bit.into(),
            check_mask_bit: args.check_mask_bit.into(),
        };

        let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: "cpu_vram_blit_compute_pass".into(),
            timestamp_writes: None,
        });

        compute_pass.set_pipeline(&self.pipeline);
        compute_pass.set_bind_group(0, &self.bind_group_0, &[]);
        compute_pass.set_bind_group(1, &bind_group_1, &[]);
        compute_pass.set_push_constants(0, bytemuck::cast_slice(&[shader_args]));

        let x_groups =
            args.width / Self::WORKGROUP_SIZE + u32::from(args.width % Self::WORKGROUP_SIZE != 0);
        let y_groups =
            args.height / Self::WORKGROUP_SIZE + u32::from(args.height % Self::WORKGROUP_SIZE != 0);
        compute_pass.dispatch_workgroups(x_groups, y_groups, 1);
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct VramSyncVertex {
    position: [i32; 2],
}

impl VramSyncVertex {
    const ATTRIBUTES: [VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Sint32x2];

    const LAYOUT: VertexBufferLayout<'static> = VertexBufferLayout {
        array_stride: mem::size_of::<Self>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

#[derive(Debug)]
struct NativeScaledSyncPipeline {
    bind_group: BindGroup,
    pipeline: RenderPipeline,
}

impl NativeScaledSyncPipeline {
    fn new(device: &Device, native_vram: &Texture, resolution_scale: u32) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "native_scaled_sync_bind_group_layout".into(),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::StorageTexture {
                        access: StorageTextureAccess::ReadOnly,
                        format: native_vram.format(),
                        view_dimension: TextureViewDimension::D2,
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
            ],
        });

        let native_vram_view = native_vram.create_view(&TextureViewDescriptor::default());
        let resolution_scale_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: "native_scaled_sync_buffer".into(),
            contents: &resolution_scale.to_le_bytes(),
            usage: BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "native_scaled_sync_bind_group".into(),
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&native_vram_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &resolution_scale_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "native_scaled_sync_pipeline_layout".into(),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("wgpuhardware/vramsync.wgsl"));
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: "native_scaled_sync_pipeline".into(),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[VramSyncVertex::LAYOUT],
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
                module: &shader,
                entry_point: "native_to_scaled",
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });

        Self { bind_group, pipeline }
    }

    fn prepare(&self, device: &Device, position: [u32; 2], size: [u32; 2]) -> Buffer {
        let position = position.map(|n| n as i32);
        let size = size.map(|n| n as i32);

        let vertices = [
            VramSyncVertex { position: [position[0], position[1]] },
            VramSyncVertex { position: [position[0] + size[0], position[1]] },
            VramSyncVertex { position: [position[0], position[1] + size[1]] },
            VramSyncVertex { position: [position[0] + size[0], position[1] + size[1]] },
        ];

        device.create_buffer_init(&BufferInitDescriptor {
            label: "native_scaled_sync_vertex_buffer".into(),
            contents: bytemuck::cast_slice(&vertices),
            usage: BufferUsages::VERTEX,
        })
    }

    fn draw<'rpass>(&'rpass self, buffer: &'rpass Buffer, render_pass: &mut RenderPass<'rpass>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, buffer.slice(..));
        render_pass.draw(0..4, 0..1);
    }
}

#[derive(Debug)]
pub struct WgpuRasterizer {
    device: Rc<Device>,
    queue: Rc<Queue>,
    resolution_scale: u32,
    scaled_vram: Texture,
    native_vram: Texture,
    frame_textures: HashMap<FrameSize, Texture>,
    clear_pipeline: ClearPipeline,
    cpu_vram_blit_pipeline: CpuVramBlitPipeline,
    native_scaled_sync_pipeline: NativeScaledSyncPipeline,
    command_buffers: Vec<CommandBuffer>,
}

impl WgpuRasterizer {
    pub fn new(device: Rc<Device>, queue: Rc<Queue>, resolution_scale: u32) -> Self {
        log::info!("Creating wgpu hardware rasterizer with resolution scale {resolution_scale}");

        let scaled_vram = device.create_texture(&TextureDescriptor {
            label: "scaled_vram_texture".into(),
            size: Extent3d {
                width: resolution_scale * VRAM_WIDTH,
                height: resolution_scale * VRAM_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::COPY_SRC
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::STORAGE_BINDING
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[TextureFormat::Rgba8UnormSrgb],
        });

        let native_vram = device.create_texture(&TextureDescriptor {
            label: "native_vram_texture".into(),
            size: Extent3d { width: VRAM_WIDTH, height: VRAM_HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            // R32 because storage textures don't support R16
            format: TextureFormat::R32Uint,
            usage: TextureUsages::COPY_DST
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let clear_pipeline = ClearPipeline::new(&device, TextureFormat::Rgba8Unorm);

        let cpu_vram_blit_pipeline = CpuVramBlitPipeline::new(&device, &native_vram);

        let native_scaled_sync_pipeline =
            NativeScaledSyncPipeline::new(&device, &native_vram, resolution_scale);

        Self {
            device,
            queue,
            resolution_scale,
            scaled_vram,
            native_vram,
            frame_textures: HashMap::with_capacity(20),
            clear_pipeline,
            cpu_vram_blit_pipeline,
            native_scaled_sync_pipeline,
            command_buffers: Vec::with_capacity(64),
        }
    }

    fn get_and_clear_frame(
        &mut self,
        frame_size: FrameSize,
        command_buffers: &mut Vec<CommandBuffer>,
    ) -> &Texture {
        let frame = get_or_create_frame_texture(
            &self.device,
            frame_size,
            self.resolution_scale,
            &mut self.frame_textures,
        );

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());
        self.clear_pipeline.draw(&frame, &mut encoder);
        command_buffers.push(encoder.finish());

        frame
    }
}

impl RasterizerInterface for WgpuRasterizer {
    fn draw_triangle(&mut self, args: DrawTriangleArgs, draw_settings: &DrawSettings) {}

    fn draw_line(&mut self, args: DrawLineArgs, draw_settings: &DrawSettings) {}

    fn draw_rectangle(&mut self, args: DrawRectangleArgs, draw_settings: &DrawSettings) {}

    fn vram_fill(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {}

    fn cpu_to_vram_blit(&mut self, args: CpuVramBlitArgs, data: &[u16]) {
        // TODO scaled/native sync

        let scaled_vram_view = self.scaled_vram.create_view(&TextureViewDescriptor::default());
        let bind_group_1 = self.cpu_vram_blit_pipeline.prepare(&self.device, &args, data);
        let sync_vertex_buffer = self.native_scaled_sync_pipeline.prepare(
            &self.device,
            [args.x, args.y],
            [args.width, args.height],
        );

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());
        self.cpu_vram_blit_pipeline.dispatch(args, bind_group_1, &mut encoder);

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "native_scaled_sync_render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &scaled_vram_view,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
                })],
                ..RenderPassDescriptor::default()
            });
            self.native_scaled_sync_pipeline.draw(&sync_vertex_buffer, &mut render_pass);
        }

        self.command_buffers.push(encoder.finish());
    }

    fn vram_to_cpu_blit(&mut self, x: u32, y: u32, width: u32, height: u32, out: &mut Vec<u16>) {
        log::warn!("VRAM-to-CPU blit: ({x}, {y}) size ({width}, {height})");
    }

    fn vram_to_vram_blit(&mut self, args: VramVramBlitArgs) {
        log::warn!(
            "VRAM-to-VRAM blit: ({}, {}) to ({}, {}) size ({}, {})",
            args.source_x,
            args.source_y,
            args.dest_x,
            args.dest_y,
            args.width,
            args.height
        );
    }

    fn generate_frame_texture(
        &mut self,
        registers: &Registers,
        wgpu_resources: &mut WgpuResources,
    ) -> &Texture {
        wgpu_resources.queued_command_buffers.extend(self.command_buffers.drain(..));

        if wgpu_resources.display_config.dump_vram {
            return &self.scaled_vram;
        }

        let (frame_coords, frame_size) =
            rasterizer::compute_frame_location(registers, wgpu_resources.display_config);
        let Some(frame_coords) = frame_coords else {
            return self
                .get_and_clear_frame(frame_size, &mut wgpu_resources.queued_command_buffers);
        };

        if !registers.display_enabled {
            return self
                .get_and_clear_frame(frame_size, &mut wgpu_resources.queued_command_buffers);
        }

        let frame = get_or_create_frame_texture(
            &self.device,
            frame_size,
            self.resolution_scale,
            &mut self.frame_textures,
        );

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());
        self.clear_pipeline.draw(&frame, &mut encoder);

        // TODO bounds check
        let source_x = frame_coords.frame_x + frame_coords.display_x_offset;
        let source_y = frame_coords.frame_y + frame_coords.display_y_offset;
        encoder.copy_texture_to_texture(
            ImageCopyTexture {
                texture: &self.scaled_vram,
                mip_level: 0,
                origin: Origin3d {
                    x: self.resolution_scale * source_x,
                    y: self.resolution_scale * source_y,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            ImageCopyTexture {
                texture: &frame,
                mip_level: 0,
                origin: Origin3d {
                    x: self.resolution_scale * frame_coords.display_x_start,
                    y: self.resolution_scale * frame_coords.display_y_start,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: frame_coords.display_width,
                height: frame_coords.display_height,
                depth_or_array_layers: 1,
            },
        );

        wgpu_resources.queued_command_buffers.push(encoder.finish());

        &frame
    }

    fn clone_vram(&self) -> Vram {
        todo!("clone VRAM")
    }
}

fn get_or_create_frame_texture<'a>(
    device: &Device,
    frame_size: FrameSize,
    resolution_scale: u32,
    frame_textures: &'a mut HashMap<FrameSize, Texture>,
) -> &'a Texture {
    frame_textures.entry(frame_size).or_insert_with(|| {
        device.create_texture(&TextureDescriptor {
            label: "frame_texture".into(),
            size: Extent3d {
                width: resolution_scale * frame_size.width,
                height: resolution_scale * frame_size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::COPY_DST
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[TextureFormat::Rgba8UnormSrgb],
        })
    })
}
