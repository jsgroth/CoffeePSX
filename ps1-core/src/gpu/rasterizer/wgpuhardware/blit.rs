use crate::gpu::rasterizer::wgpuhardware::{VRAM_HEIGHT, VRAM_WIDTH};
use crate::gpu::rasterizer::{CpuVramBlitArgs, VramVramBlitArgs};
use crate::gpu::Color;
use bytemuck::{Pod, Zeroable};
use std::{iter, mem};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferBinding,
    BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandBuffer, CommandEncoderDescriptor, ComputePass, ComputePipeline,
    ComputePipelineDescriptor, Device, FragmentState, FrontFace, Maintain, MapMode,
    MultisampleState, PipelineCompilationOptions, PipelineLayoutDescriptor, PolygonMode,
    PrimitiveState, PrimitiveTopology, PushConstantRange, Queue, RenderPass, RenderPipeline,
    RenderPipelineDescriptor, ShaderStages, StorageTextureAccess, Texture, TextureFormat,
    TextureViewDescriptor, TextureViewDimension, VertexAttribute, VertexBufferLayout, VertexState,
    VertexStepMode,
};

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
pub struct CpuVramBlitPipeline {
    ram_buffer: Vec<u32>,
    bind_group_0: BindGroup,
    bind_group_layout_1: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl CpuVramBlitPipeline {
    // Must match X/Y workgroup size in shader
    const WORKGROUP_SIZE: u32 = 16;

    pub fn new(device: &Device, native_vram: &Texture) -> Self {
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("cpuvramblit.wgsl"));
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

    pub fn prepare(
        &mut self,
        device: &Device,
        args: &CpuVramBlitArgs,
        buffer: &[u16],
    ) -> BindGroup {
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

    pub fn dispatch<'cpass>(
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
        compute_pass.set_bind_group(1, bind_group_1, &[]);
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
struct VramCpuBlitArgs {
    position: [u32; 2],
    size: [u32; 2],
}

#[derive(Debug)]
pub struct VramCpuBlitPipeline {
    blit_buffer: Buffer,
    map_buffer: Buffer,
    bind_group: BindGroup,
    pipeline: ComputePipeline,
}

impl VramCpuBlitPipeline {
    const WORKGROUP_SIZE: u32 = 16;

    pub fn new(device: &Device, native_vram: &Texture) -> Self {
        let blit_buffer = device.create_buffer(&BufferDescriptor {
            label: "vram_cpu_blit_buffer".into(),
            size: (4 * VRAM_WIDTH * VRAM_HEIGHT).into(),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let map_buffer = device.create_buffer(&BufferDescriptor {
            label: "vram_cpu_map_buffer".into(),
            size: (4 * VRAM_WIDTH * VRAM_HEIGHT).into(),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "vram_cpu_blit_bind_group_layout".into(),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::StorageTexture {
                        access: StorageTextureAccess::ReadOnly,
                        format: native_vram.format(),
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let native_vram_view = native_vram.create_view(&TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "vram_cpu_blit_bind_group".into(),
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&native_vram_view),
                },
                BindGroupEntry { binding: 1, resource: blit_buffer.as_entire_binding() },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "vram_cpu_blit_pipeline_layout".into(),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[PushConstantRange {
                stages: ShaderStages::COMPUTE,
                range: 0..mem::size_of::<VramCpuBlitArgs>() as u32,
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("vramcpublit.wgsl"));
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: "vram_cpu_blit_pipeline".into(),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "vram_cpu_blit",
            compilation_options: PipelineCompilationOptions::default(),
        });

        Self { blit_buffer, map_buffer, bind_group, pipeline }
    }

    pub fn dispatch<'cpass>(
        &'cpass self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        compute_pass: &mut ComputePass<'cpass>,
    ) {
        let args = VramCpuBlitArgs { position: [x, y], size: [width, height] };

        compute_pass.set_pipeline(&self.pipeline);
        compute_pass.set_push_constants(0, bytemuck::cast_slice(&[args]));
        compute_pass.set_bind_group(0, &self.bind_group, &[]);

        let x_workgroups =
            width / Self::WORKGROUP_SIZE + u32::from(width % Self::WORKGROUP_SIZE != 0);
        let y_workgroups =
            height / Self::WORKGROUP_SIZE + u32::from(height % Self::WORKGROUP_SIZE != 0);
        compute_pass.dispatch_workgroups(x_workgroups, y_workgroups, 1);
    }

    pub fn copy_blit_output(
        &self,
        device: &Device,
        queue: &Queue,
        width: u32,
        height: u32,
        previous_commands: impl Iterator<Item = CommandBuffer>,
        out: &mut Vec<u16>,
    ) {
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());

        let copy_len: u64 = (4 * width * height).into();
        encoder.copy_buffer_to_buffer(&self.blit_buffer, 0, &self.map_buffer, 0, copy_len);

        queue.submit(previous_commands.chain(iter::once(encoder.finish())));

        let map_buffer_slice = self.map_buffer.slice(0..copy_len);
        map_buffer_slice.map_async(MapMode::Read, Result::unwrap);
        device.poll(Maintain::Wait);

        {
            let map_buffer_view = map_buffer_slice.get_mapped_range();
            for chunk in map_buffer_view.chunks_exact(4) {
                out.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
        }

        self.map_buffer.unmap();
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct ShaderVramCopyArgs {
    source: [u32; 2],
    destination: [u32; 2],
    size: [u32; 2],
    force_mask_bit: u32,
    check_mask_bit: u32,
}

impl ShaderVramCopyArgs {
    fn new(args: &VramVramBlitArgs) -> Self {
        Self {
            source: [args.source_x, args.source_y],
            destination: [args.dest_x, args.dest_y],
            size: [args.width, args.height],
            force_mask_bit: args.force_mask_bit.into(),
            check_mask_bit: args.check_mask_bit.into(),
        }
    }
}

#[derive(Debug)]
pub struct VramCopyPipeline {
    bind_group: BindGroup,
    pipeline: ComputePipeline,
}

impl VramCopyPipeline {
    const X_WORKGROUP_SIZE: u32 = 16;

    pub fn new(device: &Device, native_vram: &Texture) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "vram_copy_bind_group_layout".into(),
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

        let native_vram_view = native_vram.create_view(&TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "vram_copy_bind_group".into(),
            layout: &bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&native_vram_view),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "vram_copy_pipeline_layout".into(),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[PushConstantRange {
                stages: ShaderStages::COMPUTE,
                range: 0..mem::size_of::<ShaderVramCopyArgs>() as u32,
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("vramcopy.wgsl"));
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: "vram_copy_pipeline".into(),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "vram_copy",
            compilation_options: PipelineCompilationOptions::default(),
        });

        Self { bind_group, pipeline }
    }

    pub fn dispatch<'cpass>(
        &'cpass self,
        args: &VramVramBlitArgs,
        compute_pass: &mut ComputePass<'cpass>,
    ) {
        let vram_copy_args = ShaderVramCopyArgs::new(args);

        compute_pass.set_pipeline(&self.pipeline);
        compute_pass.set_push_constants(0, bytemuck::cast_slice(&[vram_copy_args]));
        compute_pass.set_bind_group(0, &self.bind_group, &[]);

        let x_workgroups = args.width / Self::X_WORKGROUP_SIZE
            + u32::from(args.width % Self::X_WORKGROUP_SIZE != 0);
        compute_pass.dispatch_workgroups(x_workgroups, args.height, 1);
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
pub struct VramFillPipeline {
    bind_group: BindGroup,
    pipeline: ComputePipeline,
}

impl VramFillPipeline {
    const WORKGROUP_SIZE: u32 = 16;

    pub fn new(device: &Device, native_vram: &Texture) -> Self {
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("vramfill.wgsl"));
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: "vram_fill_pipeline".into(),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "vram_fill",
            compilation_options: PipelineCompilationOptions::default(),
        });

        Self { bind_group, pipeline }
    }

    pub fn dispatch<'cpass>(
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
pub struct NativeScaledSyncPipeline {
    bind_group: BindGroup,
    pipeline: RenderPipeline,
}

impl NativeScaledSyncPipeline {
    pub fn new(device: &Device, native_vram: &Texture, resolution_scale: u32) -> Self {
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("vramsync.wgsl"));
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

    #[allow(clippy::unused_self)]
    pub fn prepare(&self, device: &Device, position: [u32; 2], size: [u32; 2]) -> Buffer {
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

    pub fn draw<'rpass>(
        &'rpass self,
        buffer: &'rpass Buffer,
        render_pass: &mut RenderPass<'rpass>,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, buffer.slice(..));
        render_pass.draw(0..4, 0..1);
    }
}
