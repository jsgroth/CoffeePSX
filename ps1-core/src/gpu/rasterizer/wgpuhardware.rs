use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::{
    ClearPipeline, CpuVramBlitArgs, DrawLineArgs, DrawRectangleArgs, DrawTriangleArgs, FrameSize,
    RasterizerInterface, TriangleShading, VramVramBlitArgs,
};
use crate::gpu::registers::Registers;
use crate::gpu::{rasterizer, Color, Vertex, Vram, WgpuResources};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::mem;
use std::ops::Range;
use std::rc::Rc;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferBinding,
    BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandBuffer, CommandEncoder, CommandEncoderDescriptor, ComputePass, ComputePassDescriptor,
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct ShaderDrawSettings {
    draw_area_top_left: [i32; 2],
    draw_area_bottom_right: [i32; 2],
    force_mask_bit: u32,
}

impl ShaderDrawSettings {
    fn new(draw_settings: &DrawSettings, resolution_scale: u32) -> Self {
        let resolution_scale = resolution_scale as i32;

        Self {
            draw_area_top_left: [
                resolution_scale * draw_settings.draw_area_top_left.x,
                resolution_scale * draw_settings.draw_area_top_left.y,
            ],
            draw_area_bottom_right: [
                resolution_scale * (draw_settings.draw_area_bottom_right.x + 1),
                resolution_scale * (draw_settings.draw_area_bottom_right.y + 1),
            ],
            force_mask_bit: draw_settings.force_mask_bit.into(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct UntexturedVertex {
    position: [i32; 2],
    color: [u32; 3],
}

impl UntexturedVertex {
    const ATTRIBUTES: [VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Sint32x2, 1 => Uint32x3];

    const LAYOUT: VertexBufferLayout<'static> = VertexBufferLayout {
        array_stride: mem::size_of::<Self>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

#[derive(Debug)]
struct DrawBatch {
    draw_settings: DrawSettings,
    start: u32,
    end: u32,
}

#[derive(Debug)]
struct UntexturedOpaqueTrianglePipeline {
    ram_vertex_buffer: Vec<UntexturedVertex>,
    vertex_buffer: Buffer,
    pipeline: RenderPipeline,
    batches: Vec<DrawBatch>,
}

impl UntexturedOpaqueTrianglePipeline {
    const MAX_VERTICES: u64 = 15000;

    fn new(device: &Device, draw_shader: &ShaderModule) -> Self {
        let vertex_buffer = device.create_buffer(&BufferDescriptor {
            label: "untextured_opaque_triangle_vertex_buffer".into(),
            size: Self::MAX_VERTICES * mem::size_of::<UntexturedVertex>() as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::VERTEX,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "untextured_opaque_triangle_pipeline_layout".into(),
            bind_group_layouts: &[],
            push_constant_ranges: &[PushConstantRange {
                stages: ShaderStages::FRAGMENT,
                range: 0..mem::size_of::<ShaderDrawSettings>() as u32,
            }],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: "untextured_opaque_triangle_pipeline".into(),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: draw_shader,
                entry_point: "vs_untextured",
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[UntexturedVertex::LAYOUT],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
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
                module: draw_shader,
                entry_point: "fs_untextured_opaque",
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });

        Self {
            ram_vertex_buffer: Vec::with_capacity(Self::MAX_VERTICES as usize),
            vertex_buffer,
            pipeline,
            batches: Vec::with_capacity(Self::MAX_VERTICES as usize),
        }
    }

    fn add_triangle(
        &mut self,
        vertices: [Vertex; 3],
        shading: TriangleShading,
        draw_settings: &DrawSettings,
    ) {
        let draw_offset = draw_settings.draw_offset;

        if !self.batches.last().is_some_and(|batch| &batch.draw_settings == draw_settings) {
            let start = self.batches.last().map_or(0, |batch| batch.end);
            self.batches.push(DrawBatch {
                draw_settings: draw_settings.clone(),
                start,
                end: start,
            });
        }

        let positions = vertices.map(|vertex| [vertex.x + draw_offset.x, vertex.y + draw_offset.y]);

        let colors = match shading {
            TriangleShading::Flat(color) => [color; 3],
            TriangleShading::Gouraud(colors) => colors,
        };

        for (position, color) in positions.into_iter().zip(colors) {
            self.ram_vertex_buffer.push(UntexturedVertex {
                position,
                color: [color.r.into(), color.g.into(), color.b.into()],
            });
        }

        self.batches.last_mut().unwrap().end += 3;
    }

    fn prepare(&mut self, queue: &Queue) {
        log::debug!(
            "Preparing to draw {} untextured opaque triangle vertices",
            self.ram_vertex_buffer.len()
        );

        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.ram_vertex_buffer));
        self.ram_vertex_buffer.clear();
    }

    fn draw<'rpass>(&'rpass mut self, resolution_scale: u32, render_pass: &mut RenderPass<'rpass>) {
        log::debug!("Executing {} untextured opaque triangle batches", self.batches.len());

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

        for batch in self.batches.drain(..) {
            let draw_settings = ShaderDrawSettings::new(&batch.draw_settings, resolution_scale);

            render_pass.set_push_constants(
                ShaderStages::FRAGMENT,
                0,
                bytemuck::cast_slice(&[draw_settings]),
            );
            render_pass.draw(batch.start..batch.end, 0..1);
        }
    }
}

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

    fn dispatch<'cpass>(
        &'cpass self,
        args: &CpuVramBlitArgs,
        bind_group_1: &'cpass BindGroup,
        compute_pass: &mut ComputePass<'cpass>,
    ) {
        let shader_args = ShaderCpuVramBlitArgs {
            position: [args.x, args.y],
            size: [args.width, args.height],
            force_mask_bit: args.force_mask_bit.into(),
            check_mask_bit: args.check_mask_bit.into(),
        };

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
struct ShaderVramFillArgs {
    position: [u32; 2],
    size: [u32; 2],
    color: u32,
}

#[derive(Debug)]
struct VramFillPipeline {
    bind_group: BindGroup,
    pipeline: ComputePipeline,
}

impl VramFillPipeline {
    const WORKGROUP_SIZE: u32 = 16;

    fn new(device: &Device, native_vram: &Texture) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "vram_fill_bind_group_layout".into(),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::StorageTexture {
                    access: StorageTextureAccess::WriteOnly,
                    format: native_vram.format(),
                    view_dimension: TextureViewDimension::D2,
                },
                count: None,
            }],
        });

        let native_vram_view = native_vram.create_view(&TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "vram_fill_bind_group".into(),
            layout: &bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&native_vram_view),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "vram_fill_pipeline_layout".into(),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[PushConstantRange {
                stages: ShaderStages::COMPUTE,
                range: 0..mem::size_of::<ShaderVramFillArgs>() as u32,
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("wgpuhardware/vramfill.wgsl"));
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: "vram_fill_pipeline".into(),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "vram_fill",
            compilation_options: PipelineCompilationOptions::default(),
        });

        Self { bind_group, pipeline }
    }

    fn dispatch<'cpass>(
        &'cpass self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        color: Color,
        compute_pass: &mut ComputePass<'cpass>,
    ) {
        let args = ShaderVramFillArgs {
            position: [x, y],
            size: [width, height],
            color: u32::from(color.r >> 3)
                | (u32::from(color.g >> 3) << 5)
                | (u32::from(color.b >> 3) << 10),
        };

        compute_pass.set_pipeline(&self.pipeline);
        compute_pass.set_bind_group(0, &self.bind_group, &[]);
        compute_pass.set_push_constants(0, bytemuck::cast_slice(&[args]));

        let x_workgroups =
            width / Self::WORKGROUP_SIZE + u32::from(width % Self::WORKGROUP_SIZE != 0);
        let y_workgroups =
            height / Self::WORKGROUP_SIZE + u32::from(height % Self::WORKGROUP_SIZE != 0);
        compute_pass.dispatch_workgroups(x_workgroups, y_workgroups, 1);
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
enum DrawCommand {
    DrawTriangle { args: DrawTriangleArgs, draw_settings: DrawSettings },
    CpuVramBlit { args: CpuVramBlitArgs, buffer_bind_group: BindGroup, sync_vertex_buffer: Buffer },
    VramFill { x: u32, y: u32, width: u32, height: u32, color: Color, sync_vertex_buffer: Buffer },
}

impl DrawCommand {
    fn can_share_compute_pass(&self) -> bool {
        matches!(self, Self::CpuVramBlit { .. } | Self::VramFill { .. })
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
    untextured_opaque_triangle_pipeline: UntexturedOpaqueTrianglePipeline,
    cpu_vram_blit_pipeline: CpuVramBlitPipeline,
    vram_fill_pipeline: VramFillPipeline,
    native_scaled_sync_pipeline: NativeScaledSyncPipeline,
    draw_commands: Vec<DrawCommand>,
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

        let draw_shader =
            device.create_shader_module(wgpu::include_wgsl!("wgpuhardware/draw.wgsl"));
        let untextured_opaque_triangle_pipeline =
            UntexturedOpaqueTrianglePipeline::new(&device, &draw_shader);

        let cpu_vram_blit_pipeline = CpuVramBlitPipeline::new(&device, &native_vram);

        let vram_fill_pipeline = VramFillPipeline::new(&device, &native_vram);

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
            untextured_opaque_triangle_pipeline,
            cpu_vram_blit_pipeline,
            vram_fill_pipeline,
            native_scaled_sync_pipeline,
            draw_commands: Vec::with_capacity(2000),
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

    fn flush_draw_commands(&mut self) -> Option<CommandBuffer> {
        if self.draw_commands.is_empty() {
            return None;
        }

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());

        let mut i = 0;
        while let Some(command) = self.draw_commands.get(i) {
            match command {
                DrawCommand::DrawTriangle { .. } => {
                    let mut j = i + 1;
                    while j < self.draw_commands.len()
                        && matches!(&self.draw_commands[j], DrawCommand::DrawTriangle { .. })
                    {
                        j += 1;
                    }

                    self.execute_draw_triangles(i..j, &mut encoder);

                    i = j;
                }
                DrawCommand::CpuVramBlit { .. } | DrawCommand::VramFill { .. } => {
                    let mut j = i + 1;
                    while j < self.draw_commands.len()
                        && self.draw_commands[j].can_share_compute_pass()
                    {
                        j += 1;
                    }

                    self.execute_cpu_vram_blits(i..j, &mut encoder);

                    i = j;
                }
            }
        }

        self.draw_commands.clear();

        Some(encoder.finish())
    }

    fn execute_draw_triangles(
        &mut self,
        draw_command_range: Range<usize>,
        encoder: &mut CommandEncoder,
    ) {
        log::debug!(
            "Executing {} draw triangle commands",
            draw_command_range.end - draw_command_range.start
        );

        for draw_command in &self.draw_commands[draw_command_range.clone()] {
            let DrawCommand::DrawTriangle { args, draw_settings } = draw_command else { continue };

            self.untextured_opaque_triangle_pipeline.add_triangle(
                args.vertices,
                args.shading,
                draw_settings,
            );
        }

        self.untextured_opaque_triangle_pipeline.prepare(&self.queue);

        let scaled_vram_view = self.scaled_vram.create_view(&TextureViewDescriptor::default());
        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "draw_triangles_render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &scaled_vram_view,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
                })],
                ..RenderPassDescriptor::default()
            });

            self.untextured_opaque_triangle_pipeline.draw(self.resolution_scale, &mut render_pass);
        }
    }

    fn execute_cpu_vram_blits(
        &self,
        draw_command_range: Range<usize>,
        encoder: &mut CommandEncoder,
    ) {
        {
            let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: "cpu_vram_blit_compute_pass".into(),
                timestamp_writes: None,
            });

            for draw_command in &self.draw_commands[draw_command_range.clone()] {
                match draw_command {
                    DrawCommand::CpuVramBlit { args, buffer_bind_group, .. } => {
                        self.cpu_vram_blit_pipeline.dispatch(
                            args,
                            buffer_bind_group,
                            &mut compute_pass,
                        );
                    }
                    &DrawCommand::VramFill { x, y, width, height, color, .. } => {
                        self.vram_fill_pipeline.dispatch(
                            x,
                            y,
                            width,
                            height,
                            color,
                            &mut compute_pass,
                        );
                    }
                    DrawCommand::DrawTriangle { .. } => {}
                }
            }
        }

        let scaled_vram_view = self.scaled_vram.create_view(&TextureViewDescriptor::default());
        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "cpu_vram_blit_render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &scaled_vram_view,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
                })],
                ..RenderPassDescriptor::default()
            });

            for draw_command in &self.draw_commands[draw_command_range] {
                match draw_command {
                    DrawCommand::CpuVramBlit { sync_vertex_buffer, .. }
                    | DrawCommand::VramFill { sync_vertex_buffer, .. } => {
                        self.native_scaled_sync_pipeline.draw(sync_vertex_buffer, &mut render_pass);
                    }
                    DrawCommand::DrawTriangle { .. } => {}
                }
            }
        }
    }
}

impl RasterizerInterface for WgpuRasterizer {
    fn draw_triangle(&mut self, args: DrawTriangleArgs, draw_settings: &DrawSettings) {
        self.draw_commands
            .push(DrawCommand::DrawTriangle { args, draw_settings: draw_settings.clone() });
    }

    fn draw_line(&mut self, _args: DrawLineArgs, _draw_settings: &DrawSettings) {}

    fn draw_rectangle(&mut self, _args: DrawRectangleArgs, _draw_settings: &DrawSettings) {}

    fn vram_fill(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {
        // TODO scaled/native sync

        let sync_vertex_buffer =
            self.native_scaled_sync_pipeline.prepare(&self.device, [x, y], [width, height]);

        self.draw_commands.push(DrawCommand::VramFill {
            x,
            y,
            width,
            height,
            color,
            sync_vertex_buffer,
        });
    }

    fn cpu_to_vram_blit(&mut self, args: CpuVramBlitArgs, data: &[u16]) {
        // TODO scaled/native sync

        let buffer_bind_group = self.cpu_vram_blit_pipeline.prepare(&self.device, &args, data);
        let sync_vertex_buffer = self.native_scaled_sync_pipeline.prepare(
            &self.device,
            [args.x, args.y],
            [args.width, args.height],
        );

        self.draw_commands.push(DrawCommand::CpuVramBlit {
            args,
            buffer_bind_group,
            sync_vertex_buffer,
        });
    }

    fn vram_to_cpu_blit(&mut self, x: u32, y: u32, width: u32, height: u32, _out: &mut Vec<u16>) {
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
        if let Some(command_buffer) = self.flush_draw_commands() {
            wgpu_resources.queued_command_buffers.push(command_buffer);
        }

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
                width: self.resolution_scale * frame_coords.display_width,
                height: self.resolution_scale * frame_coords.display_height,
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
