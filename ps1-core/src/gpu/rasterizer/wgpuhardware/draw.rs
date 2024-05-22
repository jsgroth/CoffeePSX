use crate::gpu::gp0::{
    DrawSettings, SemiTransparencyMode, TextureColorDepthBits, TexturePage, TextureWindow,
};
use crate::gpu::rasterizer::{
    DrawRectangleArgs, DrawTriangleArgs, RectangleTextureMapping, TextureMapping,
    TextureMappingMode, TriangleShading, TriangleTextureMapping,
};
use crate::gpu::{Color, Vertex};
use bytemuck::{Pod, Zeroable};
use std::{array, mem};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendComponent, BlendFactor,
    BlendOperation, BlendState, Buffer, BufferUsages, ColorTargetState, ColorWrites, Device,
    FragmentState, FrontFace, IndexFormat, MultisampleState, PipelineCompilationOptions,
    PipelineLayout, PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology,
    PushConstantRange, RenderPass, RenderPipeline, RenderPipelineDescriptor, ShaderModule,
    ShaderStages, StorageTextureAccess, Texture, TextureFormat, TextureViewDescriptor,
    TextureViewDimension, VertexAttribute, VertexBufferLayout, VertexState, VertexStepMode,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct ShaderDrawSettings {
    force_mask_bit: u32,
    resolution_scale: u32,
}

impl ShaderDrawSettings {
    fn new(draw_settings: &DrawSettings, resolution_scale: u32) -> Self {
        Self { force_mask_bit: draw_settings.force_mask_bit.into(), resolution_scale }
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct TexturedVertex {
    position: [i32; 2],
    color: [u32; 3],
    uv: [u32; 2],
    texpage: [u32; 2],
    tex_window_mask: [u32; 2],
    tex_window_offset: [u32; 2],
    clut: [u32; 2],
    color_depth: u32,
    modulated: u32,
}

fn vertex_texpage(texpage: &TexturePage) -> [u32; 2] {
    [64 * texpage.x_base, texpage.y_base]
}

fn vertex_tex_window_mask(window: TextureWindow) -> [u32; 2] {
    [window.x_mask << 3, window.y_mask << 3]
}

fn vertex_tex_window_offset(window: TextureWindow) -> [u32; 2] {
    [window.x_offset << 3, window.y_offset << 3]
}

fn vertex_clut<const N: usize>(mapping: &TextureMapping<N>) -> [u32; 2] {
    [(16 * mapping.clut_x).into(), mapping.clut_y.into()]
}

fn vertex_color_depth(color_depth: TextureColorDepthBits) -> u32 {
    match color_depth {
        TextureColorDepthBits::Four => 0,
        TextureColorDepthBits::Eight => 1,
        TextureColorDepthBits::Fifteen => 2,
    }
}

impl TexturedVertex {
    const ATTRIBUTES: [VertexAttribute; 9] = wgpu::vertex_attr_array![
        0 => Sint32x2,
        1 => Uint32x3,
        2 => Uint32x2,
        3 => Uint32x2,
        4 => Uint32x2,
        5 => Uint32x2,
        6 => Uint32x2,
        7 => Uint32,
        8 => Uint32,
    ];

    const LAYOUT: VertexBufferLayout<'static> = VertexBufferLayout {
        array_stride: mem::size_of::<Self>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };

    fn new(
        position: [i32; 2],
        color: Color,
        u: u8,
        v: u8,
        texture_mapping: &TriangleTextureMapping,
    ) -> Self {
        Self {
            position,
            color: [color.r.into(), color.g.into(), color.b.into()],
            uv: [u.into(), v.into()],
            texpage: vertex_texpage(&texture_mapping.texpage),
            tex_window_mask: vertex_tex_window_mask(texture_mapping.window),
            tex_window_offset: vertex_tex_window_offset(texture_mapping.window),
            clut: vertex_clut(texture_mapping),
            color_depth: vertex_color_depth(texture_mapping.texpage.color_depth),
            modulated: (texture_mapping.mode == TextureMappingMode::Modulated).into(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct TexturedRectVertex {
    position: [i32; 2],
    color: [u32; 3],
    texpage: [u32; 2],
    tex_window_mask: [u32; 2],
    tex_window_offset: [u32; 2],
    clut: [u32; 2],
    color_depth: u32,
    modulated: u32,
    base_position: [i32; 2],
    base_uv: [u32; 2],
}

impl TexturedRectVertex {
    const ATTRIBUTES: [VertexAttribute; 10] = wgpu::vertex_attr_array![
        0 => Sint32x2,
        1 => Uint32x3,
        2 => Uint32x2,
        3 => Uint32x2,
        4 => Uint32x2,
        5 => Uint32x2,
        6 => Uint32,
        7 => Uint32,
        8 => Sint32x2,
        9 => Uint32x2,
    ];

    const LAYOUT: VertexBufferLayout<'static> = VertexBufferLayout {
        array_stride: mem::size_of::<Self>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };

    fn new_vertices(
        args: &DrawRectangleArgs,
        texture_mapping: &RectangleTextureMapping,
        draw_settings: &DrawSettings,
    ) -> [Self; 4] {
        let top_left = args.top_left + draw_settings.draw_offset;
        let vertices = rect_vertices(args, draw_settings.draw_offset);

        array::from_fn(|i| Self {
            position: [vertices[i].x, vertices[i].y],
            color: [args.color.r.into(), args.color.g.into(), args.color.b.into()],
            texpage: vertex_texpage(&texture_mapping.texpage),
            tex_window_mask: vertex_tex_window_mask(texture_mapping.window),
            tex_window_offset: vertex_tex_window_offset(texture_mapping.window),
            clut: vertex_clut(texture_mapping),
            color_depth: vertex_color_depth(texture_mapping.texpage.color_depth),
            modulated: (texture_mapping.mode == TextureMappingMode::Modulated).into(),
            base_position: [top_left.x, top_left.y],
            base_uv: [texture_mapping.u[0].into(), texture_mapping.v[0].into()],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawPipeline {
    UntexturedTriangle(Option<SemiTransparencyMode>),
    TexturedTriangle(Option<SemiTransparencyMode>),
    TexturedRectangle(Option<SemiTransparencyMode>),
}

#[derive(Debug)]
struct DrawBatch {
    draw_settings: DrawSettings,
    pipeline: DrawPipeline,
    start: u32,
    end: u32,
}

impl DrawBatch {
    fn matches(&self, draw_settings: &DrawSettings, pipeline: DrawPipeline) -> bool {
        draw_settings == &self.draw_settings && pipeline == self.pipeline
    }
}

#[derive(Debug)]
pub struct DrawBuffers {
    untextured_triangle: Buffer,
    textured_triangle: Buffer,
    textured_rectangle_vertex: Buffer,
    textured_rectangle_index: Buffer,
}

#[derive(Debug)]
pub struct DrawPipelines {
    untextured_buffer: Vec<UntexturedVertex>,
    untextured_opaque_pipeline: RenderPipeline,
    untextured_average_pipeline: RenderPipeline,
    untextured_add_pipeline: RenderPipeline,
    untextured_subtract_pipeline: RenderPipeline,
    untextured_add_quarter_pipeline: RenderPipeline,
    textured_buffer: Vec<TexturedVertex>,
    textured_bind_group: BindGroup,
    textured_opaque_pipeline: RenderPipeline,
    textured_average_pipeline: RenderPipeline,
    textured_add_pipeline: RenderPipeline,
    textured_subtract_pipeline_opaque: RenderPipeline,
    textured_subtract_pipeline_transparent: RenderPipeline,
    textured_add_quarter_pipeline: RenderPipeline,
    textured_rect_buffer: Vec<TexturedRectVertex>,
    textured_rect_indices: Vec<u32>,
    textured_opaque_rect_pipeline: RenderPipeline,
    textured_average_rect_pipeline: RenderPipeline,
    textured_add_rect_pipeline: RenderPipeline,
    textured_subtract_rect_pipeline_opaque: RenderPipeline,
    textured_subtract_rect_pipeline_transparent: RenderPipeline,
    textured_add_quarter_rect_pipeline: RenderPipeline,
    batches: Vec<DrawBatch>,
}

fn create_untextured_triangle_pipeline(
    device: &Device,
    draw_shader: &ShaderModule,
    fs_entry_point: &str,
    pipeline_layout: &PipelineLayout,
    blend: Option<BlendState>,
) -> RenderPipeline {
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: format!("untextured_triangle_pipeline_{fs_entry_point}").as_str().into(),
        layout: Some(pipeline_layout),
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
            entry_point: fs_entry_point,
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend,
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview: None,
    })
}

fn create_textured_pipeline(
    device: &Device,
    draw_shader: &ShaderModule,
    vertex_buffer_layout: VertexBufferLayout<'_>,
    vs_entry_point: &str,
    fs_entry_point: &str,
    pipeline_layout: &PipelineLayout,
    blend: Option<BlendState>,
) -> RenderPipeline {
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: format!("textured_draw_pipeline_{fs_entry_point}").as_str().into(),
        layout: Some(pipeline_layout),
        vertex: VertexState {
            module: draw_shader,
            entry_point: vs_entry_point,
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[vertex_buffer_layout],
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
            entry_point: fs_entry_point,
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend,
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview: None,
    })
}

fn rect_vertices(args: &DrawRectangleArgs, draw_offset: Vertex) -> [Vertex; 4] {
    let top_left = args.top_left + draw_offset;

    [
        top_left,
        top_left + Vertex::new(args.width as i32, 0),
        top_left + Vertex::new(0, args.height as i32),
        top_left + Vertex::new(args.width as i32, args.height as i32),
    ]
}

impl DrawPipelines {
    const INITIAL_BUFFER_CAPACITY: u64 = 15000;

    const AVERAGE_BLEND: BlendState = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::Src1Alpha,
            dst_factor: BlendFactor::OneMinusSrc1Alpha,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::REPLACE,
    };

    const ADDITIVE_BLEND_SINGLE_SOURCE: BlendState = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::REPLACE,
    };

    const ADDITIVE_BLEND_DUAL_SOURCE: BlendState = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::Src1Alpha,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::REPLACE,
    };

    const SUBTRACTIVE_BLEND: BlendState = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::ReverseSubtract,
        },
        alpha: BlendComponent::REPLACE,
    };

    const ADD_QUARTER_BLEND: BlendState = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::Src1Alpha,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::REPLACE,
    };

    pub fn new(
        device: &Device,
        draw_shader: &ShaderModule,
        native_vram: &Texture,
        scaled_vram_copy: &Texture,
    ) -> Self {
        let untextured_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "untextured_opaque_triangle_pipeline_layout".into(),
            bind_group_layouts: &[],
            push_constant_ranges: &[PushConstantRange {
                stages: ShaderStages::FRAGMENT,
                range: 0..mem::size_of::<ShaderDrawSettings>() as u32,
            }],
        });

        let untextured_opaque_pipeline = create_untextured_triangle_pipeline(
            device,
            draw_shader,
            "fs_untextured_opaque",
            &untextured_layout,
            None,
        );

        let untextured_average_pipeline = create_untextured_triangle_pipeline(
            device,
            draw_shader,
            "fs_untextured_average",
            &untextured_layout,
            Some(Self::AVERAGE_BLEND),
        );

        let untextured_add_pipeline = create_untextured_triangle_pipeline(
            device,
            draw_shader,
            "fs_untextured_opaque",
            &untextured_layout,
            Some(Self::ADDITIVE_BLEND_SINGLE_SOURCE),
        );

        let untextured_subtract_pipeline = create_untextured_triangle_pipeline(
            device,
            draw_shader,
            "fs_untextured_opaque",
            &untextured_layout,
            Some(Self::SUBTRACTIVE_BLEND),
        );

        let untextured_add_quarter_pipeline = create_untextured_triangle_pipeline(
            device,
            draw_shader,
            "fs_untextured_add_quarter",
            &untextured_layout,
            Some(Self::ADD_QUARTER_BLEND),
        );

        let textured_bind_group_layout =
            device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: "textured_opaque_triangle_bind_group_layout".into(),
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
                        ty: BindingType::StorageTexture {
                            access: StorageTextureAccess::ReadOnly,
                            format: scaled_vram_copy.format(),
                            view_dimension: TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            });

        let native_vram_view = native_vram.create_view(&TextureViewDescriptor::default());
        let scaled_vram_copy_view = scaled_vram_copy.create_view(&TextureViewDescriptor::default());
        let textured_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "textured_opaque_triangle_bind_group".into(),
            layout: &textured_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&native_vram_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&scaled_vram_copy_view),
                },
            ],
        });

        let textured_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "textured_opaque_triangle_pipeline_layout".into(),
            bind_group_layouts: &[&textured_bind_group_layout],
            push_constant_ranges: &[PushConstantRange {
                stages: ShaderStages::FRAGMENT,
                range: 0..mem::size_of::<ShaderDrawSettings>() as u32,
            }],
        });

        let textured_opaque_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedVertex::LAYOUT,
            "vs_textured",
            "fs_textured_opaque",
            &textured_pipeline_layout,
            None,
        );

        let textured_average_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedVertex::LAYOUT,
            "vs_textured",
            "fs_textured_average",
            &textured_pipeline_layout,
            Some(Self::AVERAGE_BLEND),
        );

        let textured_add_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedVertex::LAYOUT,
            "vs_textured",
            "fs_textured_add",
            &textured_pipeline_layout,
            Some(Self::ADDITIVE_BLEND_DUAL_SOURCE),
        );

        let textured_subtract_pipeline_opaque = create_textured_pipeline(
            device,
            draw_shader,
            TexturedVertex::LAYOUT,
            "vs_textured",
            "fs_textured_subtract_opaque_texels",
            &textured_pipeline_layout,
            None,
        );

        let textured_subtract_pipeline_transparent = create_textured_pipeline(
            device,
            draw_shader,
            TexturedVertex::LAYOUT,
            "vs_textured",
            "fs_textured_subtract_transparent_texels",
            &textured_pipeline_layout,
            Some(Self::SUBTRACTIVE_BLEND),
        );

        let textured_add_quarter_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedVertex::LAYOUT,
            "vs_textured",
            "fs_textured_add_quarter",
            &textured_pipeline_layout,
            Some(Self::ADD_QUARTER_BLEND),
        );

        let textured_opaque_rect_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedRectVertex::LAYOUT,
            "vs_textured_rect",
            "fs_textured_rect_opaque",
            &textured_pipeline_layout,
            None,
        );

        let textured_average_rect_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedRectVertex::LAYOUT,
            "vs_textured_rect",
            "fs_textured_rect_average",
            &textured_pipeline_layout,
            Some(Self::AVERAGE_BLEND),
        );

        let textured_add_rect_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedRectVertex::LAYOUT,
            "vs_textured_rect",
            "fs_textured_rect_add",
            &textured_pipeline_layout,
            Some(Self::ADDITIVE_BLEND_DUAL_SOURCE),
        );

        let textured_subtract_rect_pipeline_opaque = create_textured_pipeline(
            device,
            draw_shader,
            TexturedRectVertex::LAYOUT,
            "vs_textured_rect",
            "fs_textured_rect_subtract_opaque_texels",
            &textured_pipeline_layout,
            None,
        );

        let textured_subtract_rect_pipeline_transparent = create_textured_pipeline(
            device,
            draw_shader,
            TexturedRectVertex::LAYOUT,
            "vs_textured_rect",
            "fs_textured_rect_subtract_transparent_texels",
            &textured_pipeline_layout,
            Some(Self::SUBTRACTIVE_BLEND),
        );

        let textured_add_quarter_rect_pipeline = create_textured_pipeline(
            device,
            draw_shader,
            TexturedRectVertex::LAYOUT,
            "vs_textured_rect",
            "fs_textured_rect_add_quarter",
            &textured_pipeline_layout,
            Some(Self::ADD_QUARTER_BLEND),
        );

        Self {
            untextured_buffer: Vec::with_capacity(Self::INITIAL_BUFFER_CAPACITY as usize),
            untextured_opaque_pipeline,
            untextured_average_pipeline,
            untextured_add_pipeline,
            untextured_subtract_pipeline,
            untextured_add_quarter_pipeline,
            textured_buffer: Vec::with_capacity(Self::INITIAL_BUFFER_CAPACITY as usize),
            textured_bind_group,
            textured_opaque_pipeline,
            textured_average_pipeline,
            textured_add_pipeline,
            textured_subtract_pipeline_opaque,
            textured_subtract_pipeline_transparent,
            textured_add_quarter_pipeline,
            textured_rect_buffer: Vec::with_capacity(Self::INITIAL_BUFFER_CAPACITY as usize),
            textured_rect_indices: Vec::with_capacity(Self::INITIAL_BUFFER_CAPACITY as usize),
            textured_opaque_rect_pipeline,
            textured_average_rect_pipeline,
            textured_add_rect_pipeline,
            textured_subtract_rect_pipeline_opaque,
            textured_subtract_rect_pipeline_transparent,
            textured_add_quarter_rect_pipeline,
            batches: Vec::with_capacity(Self::INITIAL_BUFFER_CAPACITY as usize),
        }
    }

    pub fn add_triangle(&mut self, args: &DrawTriangleArgs, draw_settings: &DrawSettings) {
        let semi_transparency_mode = args.semi_transparent.then_some(args.semi_transparency_mode);
        let textured = args.texture_mapping.is_some();
        let pipeline = if textured {
            DrawPipeline::TexturedTriangle(semi_transparency_mode)
        } else {
            DrawPipeline::UntexturedTriangle(semi_transparency_mode)
        };

        // Subtractive semi-transparent textured triangles must go in their own batch
        if pipeline == DrawPipeline::TexturedTriangle(Some(SemiTransparencyMode::Subtract))
            || !self.batches.last().is_some_and(|batch| batch.matches(draw_settings, pipeline))
        {
            let start =
                (if textured { self.textured_buffer.len() } else { self.untextured_buffer.len() })
                    as u32;
            self.batches.push(DrawBatch {
                draw_settings: draw_settings.clone(),
                pipeline,
                start,
                end: start,
            });
        }

        let draw_offset = draw_settings.draw_offset;
        let positions =
            args.vertices.map(|vertex| [vertex.x + draw_offset.x, vertex.y + draw_offset.y]);

        let colors = match args.shading {
            TriangleShading::Flat(color) => [color; 3],
            TriangleShading::Gouraud(colors) => colors,
        };

        match &args.texture_mapping {
            Some(mapping) => {
                for (i, position) in positions.into_iter().enumerate() {
                    self.textured_buffer.push(TexturedVertex::new(
                        position,
                        colors[i],
                        mapping.u[i],
                        mapping.v[i],
                        mapping,
                    ));
                }
            }
            None => {
                for (position, color) in positions.into_iter().zip(colors) {
                    self.untextured_buffer.push(UntexturedVertex {
                        position,
                        color: [color.r.into(), color.g.into(), color.b.into()],
                    });
                }
            }
        }

        self.batches.last_mut().unwrap().end += 3;
    }

    pub fn add_rectangle(&mut self, args: &DrawRectangleArgs, draw_settings: &DrawSettings) {
        match &args.texture_mapping {
            Some(texture_mapping) => {
                let semi_transparency_mode =
                    args.semi_transparent.then_some(args.semi_transparency_mode);
                let pipeline = DrawPipeline::TexturedRectangle(semi_transparency_mode);

                // Subtractive semi-transparent textured rectangles must go in their own batch
                if semi_transparency_mode == Some(SemiTransparencyMode::Subtract)
                    || !self
                        .batches
                        .last()
                        .is_some_and(|batch| batch.matches(draw_settings, pipeline))
                {
                    let start = self.textured_rect_buffer.len() as u32;
                    self.batches.push(DrawBatch {
                        draw_settings: draw_settings.clone(),
                        pipeline,
                        start,
                        end: start,
                    });
                }

                let vertices =
                    TexturedRectVertex::new_vertices(args, texture_mapping, draw_settings);
                self.textured_rect_buffer.extend(vertices);

                self.batches.last_mut().unwrap().end += 4;
            }
            None => {
                let v = rect_vertices(args, Vertex::new(0, 0));
                for vertices in [[v[0], v[1], v[2]], [v[1], v[2], v[3]]] {
                    self.add_triangle(
                        &DrawTriangleArgs {
                            vertices,
                            shading: TriangleShading::Flat(args.color),
                            semi_transparent: args.semi_transparent,
                            semi_transparency_mode: args.semi_transparency_mode,
                            texture_mapping: None,
                        },
                        draw_settings,
                    );
                }
            }
        }
    }

    pub fn prepare(&mut self, device: &Device) -> DrawBuffers {
        let untextured_triangle = device.create_buffer_init(&BufferInitDescriptor {
            label: "untextured_triangle_vertex_buffer".into(),
            contents: bytemuck::cast_slice(&self.untextured_buffer),
            usage: BufferUsages::VERTEX,
        });

        let textured_triangle = device.create_buffer_init(&BufferInitDescriptor {
            label: "textured_triangle_vertex_buffer".into(),
            contents: bytemuck::cast_slice(&self.textured_buffer),
            usage: BufferUsages::VERTEX,
        });

        let textured_rectangle_vertex = device.create_buffer_init(&BufferInitDescriptor {
            label: "textured_rectangle_vertex_buffer".into(),
            contents: bytemuck::cast_slice(&self.textured_rect_buffer),
            usage: BufferUsages::VERTEX,
        });

        for i in (0..self.textured_rect_buffer.len()).step_by(4) {
            let i = i as u32;
            self.textured_rect_indices.extend([i, i + 1, i + 2, i + 1, i + 2, i + 3]);
        }

        let textured_rectangle_index = device.create_buffer_init(&BufferInitDescriptor {
            label: "textured_rectangle_index_buffer".into(),
            contents: bytemuck::cast_slice(&self.textured_rect_indices),
            usage: BufferUsages::INDEX,
        });

        self.untextured_buffer.clear();
        self.textured_buffer.clear();
        self.textured_rect_buffer.clear();
        self.textured_rect_indices.clear();

        DrawBuffers {
            untextured_triangle,
            textured_triangle,
            textured_rectangle_vertex,
            textured_rectangle_index,
        }
    }

    pub fn draw<'rpass>(
        &'rpass mut self,
        buffers: &'rpass DrawBuffers,
        resolution_scale: u32,
        render_pass: &mut RenderPass<'rpass>,
    ) {
        for batch in self.batches.drain(..) {
            let draw_settings = ShaderDrawSettings::new(&batch.draw_settings, resolution_scale);
            set_scissor_rect(render_pass, &batch.draw_settings, resolution_scale);

            match batch.pipeline {
                DrawPipeline::UntexturedTriangle(semi_transparency_mode) => {
                    let pipeline = match semi_transparency_mode {
                        Some(SemiTransparencyMode::Average) => &self.untextured_average_pipeline,
                        Some(SemiTransparencyMode::Add) => &self.untextured_add_pipeline,
                        Some(SemiTransparencyMode::Subtract) => &self.untextured_subtract_pipeline,
                        Some(SemiTransparencyMode::AddQuarter) => {
                            &self.untextured_add_quarter_pipeline
                        }
                        None => &self.untextured_opaque_pipeline,
                    };

                    render_pass.set_pipeline(pipeline);
                    render_pass.set_push_constants(
                        ShaderStages::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[draw_settings]),
                    );
                    render_pass.set_vertex_buffer(0, buffers.untextured_triangle.slice(..));

                    render_pass.draw(batch.start..batch.end, 0..1);
                }
                DrawPipeline::TexturedTriangle(Some(SemiTransparencyMode::Subtract)) => {
                    render_pass.set_pipeline(&self.textured_subtract_pipeline_opaque);
                    render_pass.set_push_constants(
                        ShaderStages::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[draw_settings]),
                    );
                    render_pass.set_bind_group(0, &self.textured_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, buffers.textured_triangle.slice(..));

                    render_pass.draw(batch.start..batch.end, 0..1);

                    render_pass.set_pipeline(&self.textured_subtract_pipeline_transparent);
                    render_pass.set_push_constants(
                        ShaderStages::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[draw_settings]),
                    );

                    render_pass.draw(batch.start..batch.end, 0..1);
                }
                DrawPipeline::TexturedTriangle(semi_transparency_mode) => {
                    let pipeline = match semi_transparency_mode {
                        Some(SemiTransparencyMode::Average) => &self.textured_average_pipeline,
                        Some(SemiTransparencyMode::Add) => &self.textured_add_pipeline,
                        Some(SemiTransparencyMode::Subtract) => unreachable!(),
                        Some(SemiTransparencyMode::AddQuarter) => {
                            &self.textured_add_quarter_pipeline
                        }
                        None => &self.textured_opaque_pipeline,
                    };

                    render_pass.set_pipeline(pipeline);
                    render_pass.set_push_constants(
                        ShaderStages::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[draw_settings]),
                    );
                    render_pass.set_bind_group(0, &self.textured_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, buffers.textured_triangle.slice(..));

                    render_pass.draw(batch.start..batch.end, 0..1);
                }
                DrawPipeline::TexturedRectangle(Some(SemiTransparencyMode::Subtract)) => {
                    render_pass.set_pipeline(&self.textured_subtract_rect_pipeline_opaque);
                    render_pass.set_push_constants(
                        ShaderStages::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[draw_settings]),
                    );
                    render_pass.set_bind_group(0, &self.textured_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, buffers.textured_rectangle_vertex.slice(..));
                    render_pass.set_index_buffer(
                        buffers.textured_rectangle_index.slice(..),
                        IndexFormat::Uint32,
                    );

                    let start_indexed = batch.start * 3 / 2;
                    let end_indexed = batch.end * 3 / 2;
                    render_pass.draw_indexed(start_indexed..end_indexed, 0, 0..1);

                    render_pass.set_pipeline(&self.textured_subtract_rect_pipeline_transparent);
                    render_pass.set_push_constants(
                        ShaderStages::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[draw_settings]),
                    );

                    render_pass.draw_indexed(start_indexed..end_indexed, 0, 0..1);
                }
                DrawPipeline::TexturedRectangle(semi_transparency_mode) => {
                    let pipeline = match semi_transparency_mode {
                        Some(SemiTransparencyMode::Average) => &self.textured_average_rect_pipeline,
                        Some(SemiTransparencyMode::Add) => &self.textured_add_rect_pipeline,
                        Some(SemiTransparencyMode::AddQuarter) => {
                            &self.textured_add_quarter_rect_pipeline
                        }
                        None => &self.textured_opaque_rect_pipeline,
                        Some(SemiTransparencyMode::Subtract) => unreachable!(),
                    };

                    render_pass.set_pipeline(pipeline);
                    render_pass.set_push_constants(
                        ShaderStages::FRAGMENT,
                        0,
                        bytemuck::cast_slice(&[draw_settings]),
                    );
                    render_pass.set_bind_group(0, &self.textured_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, buffers.textured_rectangle_vertex.slice(..));
                    render_pass.set_index_buffer(
                        buffers.textured_rectangle_index.slice(..),
                        IndexFormat::Uint32,
                    );

                    let start_indexed = batch.start * 3 / 2;
                    let end_indexed = batch.end * 3 / 2;
                    render_pass.draw_indexed(start_indexed..end_indexed, 0, 0..1);
                }
            }
        }
    }
}

fn set_scissor_rect(
    render_pass: &mut RenderPass<'_>,
    draw_settings: &DrawSettings,
    resolution_scale: u32,
) {
    let width =
        (draw_settings.draw_area_bottom_right.x - draw_settings.draw_area_top_left.x + 1) as u32;
    let height =
        (draw_settings.draw_area_bottom_right.y - draw_settings.draw_area_top_left.y + 1) as u32;

    render_pass.set_scissor_rect(
        resolution_scale * draw_settings.draw_area_top_left.x as u32,
        resolution_scale * draw_settings.draw_area_top_left.y as u32,
        resolution_scale * width,
        resolution_scale * height,
    );
}
