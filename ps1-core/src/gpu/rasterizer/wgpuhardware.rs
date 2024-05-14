use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::{
    ClearPipeline, CpuVramBlitArgs, DrawLineArgs, DrawRectangleArgs, DrawTriangleArgs, FrameSize,
    RasterizerInterface, VramVramBlitArgs,
};
use crate::gpu::registers::Registers;
use crate::gpu::{rasterizer, Color, Vram, WgpuResources};
use std::collections::HashMap;
use std::rc::Rc;
use wgpu::{
    CommandBuffer, CommandEncoderDescriptor, Device, Extent3d, ImageCopyTexture, LoadOp,
    Operations, Origin3d, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    StoreOp, Texture, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureViewDescriptor,
};

const VRAM_WIDTH: u32 = 1024;
const VRAM_HEIGHT: u32 = 512;

#[derive(Debug)]
pub struct WgpuRasterizer {
    device: Rc<Device>,
    queue: Rc<Queue>,
    resolution_scale: u32,
    scaled_vram: Texture,
    frame_textures: HashMap<FrameSize, Texture>,
    clear_pipeline: ClearPipeline,
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
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[TextureFormat::Rgba8UnormSrgb],
        });

        let clear_pipeline = ClearPipeline::new(&device, TextureFormat::Rgba8Unorm);

        Self {
            device,
            queue,
            resolution_scale,
            scaled_vram,
            frame_textures: HashMap::with_capacity(20),
            clear_pipeline,
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

    fn cpu_to_vram_blit(&mut self, args: CpuVramBlitArgs, data: &[u16]) {}

    fn vram_to_cpu_blit(&mut self, x: u32, y: u32, width: u32, height: u32, out: &mut Vec<u16>) {}

    fn vram_to_vram_blit(&mut self, args: VramVramBlitArgs) {}

    fn generate_frame_texture(
        &mut self,
        registers: &Registers,
        wgpu_resources: &mut WgpuResources,
    ) -> &Texture {
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
