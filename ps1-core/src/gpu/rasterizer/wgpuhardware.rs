use crate::gpu::gp0::DrawSettings;
use crate::gpu::rasterizer::{
    CpuVramBlitArgs, DrawLineArgs, DrawRectangleArgs, DrawTriangleArgs, RasterizerInterface,
    VramVramBlitArgs,
};
use crate::gpu::registers::Registers;
use crate::gpu::{Color, Vram, WgpuResources};
use std::rc::Rc;
use wgpu::{
    Device, Extent3d, Queue, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages,
};

const VRAM_WIDTH: u32 = 1024;
const VRAM_HEIGHT: u32 = 512;

#[derive(Debug)]
pub struct WgpuRasterizer {
    device: Rc<Device>,
    queue: Rc<Queue>,
    resolution_scale: u32,
    scaled_vram: Texture,
}

impl WgpuRasterizer {
    pub fn new(device: Rc<Device>, queue: Rc<Queue>, resolution_scale: u32) -> Self {
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
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[TextureFormat::Rgba8UnormSrgb],
        });

        Self { device, queue, resolution_scale, scaled_vram }
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
        &self.scaled_vram
    }

    fn clone_vram(&self) -> Vram {
        todo!("clone VRAM")
    }
}
