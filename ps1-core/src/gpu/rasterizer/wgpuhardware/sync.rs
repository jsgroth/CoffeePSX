use crate::gpu::Vertex;
use bytemuck::{Pod, Zeroable};
use std::mem;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, Buffer, BufferBinding, BufferBindingType,
    BufferUsages, ColorTargetState, ColorWrites, Device, FilterMode, FragmentState, FrontFace,
    MultisampleState, PipelineCompilationOptions, PipelineLayoutDescriptor, PolygonMode,
    PrimitiveState, PrimitiveTopology, RenderPass, RenderPipeline, RenderPipelineDescriptor,
    SamplerBindingType, SamplerDescriptor, ShaderStages, StorageTextureAccess, Texture,
    TextureFormat, TextureSampleType, TextureViewDescriptor, TextureViewDimension, VertexAttribute,
    VertexBufferLayout, VertexState, VertexStepMode,
};

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

        let shader = device.create_shader_module(wgpu::include_wgsl!("scaledsync.wgsl"));
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
                    blend: None,
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

#[derive(Debug)]
pub struct ScaledNativeSyncPipeline {
    bind_group: BindGroup,
    pipeline: RenderPipeline,
}

impl ScaledNativeSyncPipeline {
    pub fn new(device: &Device, scaled_vram: &Texture, resolution_scale: u32) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "scaled_native_sync_bind_group".into(),
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

        let scaled_vram_view = scaled_vram.create_view(&TextureViewDescriptor::default());

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: "scaled_native_sync_sampler".into(),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..SamplerDescriptor::default()
        });

        let resolution_scale_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: "scaled_native_sync_resolution_scale_buffer".into(),
            contents: &resolution_scale.to_le_bytes(),
            usage: BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "scaled_native_sync_bind_group".into(),
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&scaled_vram_view),
                },
                BindGroupEntry { binding: 1, resource: BindingResource::Sampler(&sampler) },
                BindGroupEntry {
                    binding: 2,
                    resource: resolution_scale_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "scaled_native_sync_pipeline_layout".into(),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("nativesync.wgsl"));
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: "scaled_native_sync_pipeline".into(),
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
                entry_point: "fs_main",
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::R32Uint,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });

        Self { bind_group, pipeline }
    }

    #[allow(clippy::unused_self)]
    pub fn prepare(&self, device: &Device, top_left: Vertex, bottom_right: Vertex) -> Buffer {
        let vertices = [
            VramSyncVertex { position: [top_left.x, top_left.y] },
            VramSyncVertex { position: [bottom_right.x, top_left.y] },
            VramSyncVertex { position: [top_left.x, bottom_right.y] },
            VramSyncVertex { position: [bottom_right.x, bottom_right.y] },
        ];

        device.create_buffer_init(&BufferInitDescriptor {
            label: "scaled_native_sync_vertex_buffer".into(),
            contents: bytemuck::cast_slice(&vertices),
            usage: BufferUsages::VERTEX,
        })
    }

    pub fn draw<'rpass>(
        &'rpass self,
        vertex_buffer: &'rpass Buffer,
        render_pass: &mut RenderPass<'rpass>,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.draw(0..4, 0..1);
    }
}
