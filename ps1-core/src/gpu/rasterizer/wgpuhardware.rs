mod blit;
mod draw;
mod hazards;
mod sync;
mod twentyfour;

use crate::api::ColorDepthBits;
use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::wgpuhardware::blit::{
    CpuVramBlitPipeline, VramCopyPipeline, VramCpuBlitPipeline, VramFillPipeline,
};
use crate::gpu::rasterizer::wgpuhardware::draw::DrawPipelines;
use crate::gpu::rasterizer::wgpuhardware::hazards::HazardTracker;
use crate::gpu::rasterizer::wgpuhardware::sync::{
    NativeScaledSyncPipeline, ScaledNativeSyncBuffers, ScaledNativeSyncPipeline,
};
use crate::gpu::rasterizer::wgpuhardware::twentyfour::TwentyFourBppPipeline;
use crate::gpu::rasterizer::{
    vertices_valid, ClearPipeline, CpuVramBlitArgs, DrawLineArgs, DrawRectangleArgs,
    DrawTriangleArgs, FrameCoords, FrameSize, RasterizerInterface, VramVramBlitArgs,
};
use crate::gpu::registers::Registers;
use crate::gpu::{rasterizer, Color, Vertex, Vram, WgpuResources};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::{cmp, iter};
use wgpu::{
    BindGroup, Buffer, BufferDescriptor, BufferUsages, CommandBuffer, CommandEncoder,
    CommandEncoderDescriptor, ComputePassDescriptor, Device, Extent3d, ImageCopyBuffer,
    ImageCopyTexture, ImageDataLayout, LoadOp, Maintain, MapMode, Operations, Origin3d, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, StoreOp, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
};

const VRAM_WIDTH: u32 = 1024;
const VRAM_HEIGHT: u32 = 512;

#[derive(Debug)]
enum DrawCommand {
    DrawTriangle { args: DrawTriangleArgs, draw_settings: DrawSettings },
    DrawRectangle { args: DrawRectangleArgs, draw_settings: DrawSettings },
    CpuVramBlit { args: CpuVramBlitArgs, buffer_bind_group: BindGroup, sync_vertex_buffer: Buffer },
    VramCopy { args: VramVramBlitArgs, sync_vertex_buffer: Buffer },
    VramFill { x: u32, y: u32, width: u32, height: u32, color: Color, sync_vertex_buffer: Buffer },
    ScaledNativeSync { buffers: ScaledNativeSyncBuffers },
}

impl DrawCommand {
    fn can_share_compute_pass(&self) -> bool {
        matches!(self, Self::CpuVramBlit { .. } | Self::VramCopy { .. } | Self::VramFill { .. })
    }
}

#[derive(Debug)]
pub struct WgpuRasterizer {
    device: Rc<Device>,
    queue: Rc<Queue>,
    resolution_scale: u32,
    scaled_vram: Texture,
    native_vram: Texture,
    frame_textures: HashMap<(FrameSize, u32), Texture>,
    hazard_tracker: HazardTracker,
    clear_pipeline: ClearPipeline,
    render_24bpp_pipeline: TwentyFourBppPipeline,
    draw_pipelines: DrawPipelines,
    cpu_vram_blit_pipeline: CpuVramBlitPipeline,
    vram_cpu_blit_pipeline: VramCpuBlitPipeline,
    vram_copy_pipeline: VramCopyPipeline,
    vram_fill_pipeline: VramFillPipeline,
    native_scaled_sync_pipeline: NativeScaledSyncPipeline,
    scaled_native_sync_pipeline: ScaledNativeSyncPipeline,
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
            usage: TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::STORAGE_BINDING
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let clear_pipeline = ClearPipeline::new(&device, TextureFormat::Rgba8Unorm);

        let render_24bpp_pipeline = TwentyFourBppPipeline::new(&device, &native_vram);

        let draw_shader =
            device.create_shader_module(wgpu::include_wgsl!("wgpuhardware/draw.wgsl"));
        let draw_pipelines = DrawPipelines::new(&device, &draw_shader, &native_vram);

        let cpu_vram_blit_pipeline = CpuVramBlitPipeline::new(&device, &native_vram);
        let vram_cpu_blit_pipeline = VramCpuBlitPipeline::new(&device, &native_vram);
        let vram_copy_pipeline = VramCopyPipeline::new(&device, &native_vram);
        let vram_fill_pipeline = VramFillPipeline::new(&device, &native_vram);

        let native_scaled_sync_pipeline =
            NativeScaledSyncPipeline::new(&device, &native_vram, resolution_scale);
        let scaled_native_sync_pipeline =
            ScaledNativeSyncPipeline::new(&device, &scaled_vram, resolution_scale);

        Self {
            device,
            queue,
            resolution_scale,
            scaled_vram,
            native_vram,
            frame_textures: HashMap::with_capacity(20),
            hazard_tracker: HazardTracker::new(),
            clear_pipeline,
            render_24bpp_pipeline,
            draw_pipelines,
            cpu_vram_blit_pipeline,
            vram_cpu_blit_pipeline,
            vram_copy_pipeline,
            vram_fill_pipeline,
            native_scaled_sync_pipeline,
            scaled_native_sync_pipeline,
            draw_commands: Vec::with_capacity(2000),
        }
    }

    pub fn copy_vram_from(&self, vram: &Vram) {
        let vram_u32: Vec<_> = vram.iter().copied().map(u32::from).collect();

        self.queue.write_texture(
            self.native_vram.as_image_copy(),
            bytemuck::cast_slice(&vram_u32),
            ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * VRAM_WIDTH),
                rows_per_image: None,
            },
            self.native_vram.size(),
        );

        let sync_vertex_buffer =
            self.native_scaled_sync_pipeline.prepare(&self.device, [0, 0], [1024, 512]);

        let scaled_vram_view = self.scaled_vram.create_view(&TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "load_state_render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &scaled_vram_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                ..RenderPassDescriptor::default()
            });

            self.native_scaled_sync_pipeline.draw(&sync_vertex_buffer, &mut render_pass);
        }

        self.queue.submit(iter::once(encoder.finish()));
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
        self.clear_pipeline.draw(frame, &mut encoder);
        command_buffers.push(encoder.finish());

        frame
    }

    fn render_24bpp(
        &mut self,
        frame_coords: FrameCoords,
        frame_size: FrameSize,
        command_buffers: &mut Vec<CommandBuffer>,
    ) -> &Texture {
        let frame =
            get_or_create_frame_texture(&self.device, frame_size, 1, &mut self.frame_textures);
        let frame_view = frame.create_view(&TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "render_24bpp_render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &frame_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                ..RenderPassDescriptor::default()
            });

            self.render_24bpp_pipeline.draw(frame_coords, &mut render_pass);
        }

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
                DrawCommand::DrawTriangle { .. } | DrawCommand::DrawRectangle { .. } => {
                    let mut j = i + 1;
                    while j < self.draw_commands.len()
                        && matches!(
                            &self.draw_commands[j],
                            DrawCommand::DrawTriangle { .. } | DrawCommand::DrawRectangle { .. }
                        )
                    {
                        j += 1;
                    }

                    self.execute_draw(i..j, &mut encoder);

                    i = j;
                }
                DrawCommand::CpuVramBlit { .. }
                | DrawCommand::VramCopy { .. }
                | DrawCommand::VramFill { .. } => {
                    let mut j = i + 1;
                    while j < self.draw_commands.len()
                        && self.draw_commands[j].can_share_compute_pass()
                    {
                        j += 1;
                    }

                    self.execute_blits(i..j, &mut encoder);

                    i = j;
                }
                DrawCommand::ScaledNativeSync { buffers } => {
                    self.execute_scaled_native_sync(buffers, &mut encoder);
                    i += 1;
                }
            }
        }

        self.draw_commands.clear();

        if let Some(bounding_box) = self.hazard_tracker.bounding_box() {
            let buffers = self.scaled_native_sync_pipeline.prepare(
                &self.device,
                bounding_box,
                self.hazard_tracker.atlas.as_ref(),
            );
            self.execute_scaled_native_sync(&buffers, &mut encoder);
            self.hazard_tracker.clear();
        }

        Some(encoder.finish())
    }

    fn execute_draw(&mut self, draw_command_range: Range<usize>, encoder: &mut CommandEncoder) {
        log::debug!("Executing {} draw commands", draw_command_range.len());

        for draw_command in &self.draw_commands[draw_command_range.clone()] {
            match draw_command {
                DrawCommand::DrawTriangle { args, draw_settings } => {
                    self.draw_pipelines.add_triangle(args, draw_settings);
                }
                DrawCommand::DrawRectangle { args, draw_settings } => {
                    self.draw_pipelines.add_rectangle(args, draw_settings);
                }
                DrawCommand::CpuVramBlit { .. }
                | DrawCommand::VramFill { .. }
                | DrawCommand::VramCopy { .. }
                | DrawCommand::ScaledNativeSync { .. } => {}
            }
        }

        let draw_buffers = self.draw_pipelines.prepare(&self.device);

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

            self.draw_pipelines.draw(&draw_buffers, self.resolution_scale, &mut render_pass);
        }
    }

    fn execute_blits(&self, draw_command_range: Range<usize>, encoder: &mut CommandEncoder) {
        log::debug!("Executing {} blit commands", draw_command_range.len());

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
                    DrawCommand::VramCopy { args, .. } => {
                        self.vram_copy_pipeline.dispatch(args, &mut compute_pass);
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
                    DrawCommand::DrawTriangle { .. }
                    | DrawCommand::DrawRectangle { .. }
                    | DrawCommand::ScaledNativeSync { .. } => {}
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
                    | DrawCommand::VramCopy { sync_vertex_buffer, .. }
                    | DrawCommand::VramFill { sync_vertex_buffer, .. } => {
                        self.native_scaled_sync_pipeline.draw(sync_vertex_buffer, &mut render_pass);
                    }
                    DrawCommand::DrawTriangle { .. }
                    | DrawCommand::DrawRectangle { .. }
                    | DrawCommand::ScaledNativeSync { .. } => {}
                }
            }
        }
    }

    fn execute_scaled_native_sync(
        &self,
        buffers: &ScaledNativeSyncBuffers,
        encoder: &mut CommandEncoder,
    ) {
        log::debug!("Syncing scaled VRAM to native");

        let native_vram_view = self.native_vram.create_view(&TextureViewDescriptor::default());
        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: "scaled_native_sync_render_pass".into(),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &native_vram_view,
                    resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
                })],
                ..RenderPassDescriptor::default()
            });

            self.scaled_native_sync_pipeline.draw(buffers, &mut render_pass);
        }
    }
}

impl RasterizerInterface for WgpuRasterizer {
    fn draw_triangle(&mut self, args: DrawTriangleArgs, draw_settings: &DrawSettings) {
        if !draw_settings.is_drawing_area_valid() {
            return;
        }

        if !vertices_valid(args.vertices[0], args.vertices[1])
            || !vertices_valid(args.vertices[1], args.vertices[2])
            || !vertices_valid(args.vertices[2], args.vertices[0])
        {
            return;
        }

        if draw_settings.check_mask_bit {
            log::warn!("Draw triangle with mask bit {args:?}");
        }

        let Some((bounding_box_top_left, bounding_box_top_right)) =
            triangle_bounding_box(&args, draw_settings)
        else {
            return;
        };
        self.hazard_tracker.mark_rendered(bounding_box_top_left, bounding_box_top_right);

        // TODO proper scaled/native sync

        self.draw_commands
            .push(DrawCommand::DrawTriangle { args, draw_settings: draw_settings.clone() });
    }

    fn draw_line(&mut self, _args: DrawLineArgs, _draw_settings: &DrawSettings) {
        log::warn!("Draw line {_args:?} {_draw_settings:?}");
    }

    fn draw_rectangle(&mut self, args: DrawRectangleArgs, draw_settings: &DrawSettings) {
        if !draw_settings.is_drawing_area_valid() {
            return;
        }

        if draw_settings.check_mask_bit {
            log::warn!("Draw rectangle with mask bit {args:?}");
        }

        let Some((bounding_box_top_left, bounding_box_top_right)) =
            rectangle_bounding_box(&args, draw_settings)
        else {
            return;
        };
        self.hazard_tracker.mark_rendered(bounding_box_top_left, bounding_box_top_right);

        // TODO proper scaled/native sync

        self.draw_commands
            .push(DrawCommand::DrawRectangle { args, draw_settings: draw_settings.clone() });
    }

    fn vram_fill(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {
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

    fn vram_to_cpu_blit(&mut self, x: u32, y: u32, width: u32, height: u32, out: &mut Vec<u16>) {
        let flush_command_buffer = self.flush_draw_commands();

        log::debug!("VRAM-to-CPU blit: position ({x}, {y}) size ({width}, {height})");

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());

        {
            let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: "vram_cpu_blit_compute_pass".into(),
                timestamp_writes: None,
            });
            self.vram_cpu_blit_pipeline.dispatch(x, y, width, height, &mut compute_pass);
        }

        self.vram_cpu_blit_pipeline.copy_blit_output(
            &self.device,
            &self.queue,
            width,
            height,
            flush_command_buffer.into_iter().chain(iter::once(encoder.finish())),
            out,
        );
    }

    fn vram_to_vram_blit(&mut self, args: VramVramBlitArgs) {
        // TODO scaled/native sync

        let sync_vertex_buffer = self.native_scaled_sync_pipeline.prepare(
            &self.device,
            [args.dest_x, args.dest_y],
            [args.width, args.height],
        );

        self.draw_commands.push(DrawCommand::VramCopy { args, sync_vertex_buffer });
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

        if registers.display_area_color_depth == ColorDepthBits::TwentyFour {
            return self.render_24bpp(
                frame_coords,
                frame_size,
                &mut wgpu_resources.queued_command_buffers,
            );
        }

        let frame = get_or_create_frame_texture(
            &self.device,
            frame_size,
            self.resolution_scale,
            &mut self.frame_textures,
        );

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());
        self.clear_pipeline.draw(frame, &mut encoder);

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
                texture: frame,
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

        frame
    }

    fn clone_vram(&mut self) -> Vram {
        let flush_command_buffer = self.flush_draw_commands();

        let vram_buffer = self.device.create_buffer(&BufferDescriptor {
            label: "vram_buffer".into(),
            size: (4 * VRAM_WIDTH * VRAM_HEIGHT).into(),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            self.native_vram.as_image_copy(),
            ImageCopyBuffer {
                buffer: &vram_buffer,
                layout: ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * VRAM_WIDTH),
                    rows_per_image: None,
                },
            },
            self.native_vram.size(),
        );

        self.queue.submit(flush_command_buffer.into_iter().chain(iter::once(encoder.finish())));

        let vram_buffer_slice = vram_buffer.slice(..);
        vram_buffer_slice.map_async(MapMode::Read, Result::unwrap);
        self.device.poll(Maintain::Wait);

        let vram_buffer_view = vram_buffer_slice.get_mapped_range();

        let mut vram = Vram::new();
        for (chunk, vram_value) in vram_buffer_view.chunks_exact(4).zip(vram.iter_mut()) {
            *vram_value = u16::from_le_bytes([chunk[0], chunk[1]]);
        }

        vram
    }
}

fn triangle_bounding_box(
    args: &DrawTriangleArgs,
    draw_settings: &DrawSettings,
) -> Option<(Vertex, Vertex)> {
    let v_x = args.vertices.map(|v| v.x + draw_settings.draw_offset.x);
    let v_y = args.vertices.map(|v| v.y + draw_settings.draw_offset.y);

    let min_x = cmp::max(draw_settings.draw_area_top_left.x, v_x.into_iter().min().unwrap());
    let max_x =
        cmp::min(draw_settings.draw_area_bottom_right.x + 1, v_x.into_iter().max().unwrap());
    let min_y = cmp::max(draw_settings.draw_area_top_left.y, v_y.into_iter().min().unwrap());
    let max_y =
        cmp::min(draw_settings.draw_area_bottom_right.y + 1, v_y.into_iter().max().unwrap());

    if min_x >= max_x || min_y >= max_y {
        return None;
    }

    Some((Vertex::new(min_x, min_y), Vertex::new(max_x, max_y)))
}

fn rectangle_bounding_box(
    args: &DrawRectangleArgs,
    draw_settings: &DrawSettings,
) -> Option<(Vertex, Vertex)> {
    let top_left = args.top_left + draw_settings.draw_offset;

    let min_x = cmp::max(draw_settings.draw_area_top_left.x, top_left.x);
    let max_x =
        cmp::min(draw_settings.draw_area_bottom_right.x + 1, top_left.x + args.width as i32);
    let min_y = cmp::max(draw_settings.draw_area_top_left.y, top_left.y);
    let max_y =
        cmp::min(draw_settings.draw_area_bottom_right.y + 1, top_left.y + args.height as i32);

    if min_x >= max_x || min_y >= max_y {
        return None;
    }

    Some((Vertex::new(min_x, min_y), Vertex::new(max_x, max_y)))
}

fn get_or_create_frame_texture<'a>(
    device: &Device,
    frame_size: FrameSize,
    resolution_scale: u32,
    frame_textures: &'a mut HashMap<(FrameSize, u32), Texture>,
) -> &'a Texture {
    frame_textures.entry((frame_size, resolution_scale)).or_insert_with(|| {
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
