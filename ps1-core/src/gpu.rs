//! PS1 GPU (Graphics Processing Unit)
//!
//! The GPU has no real 3D capabilities. Its primary capability is that it can rasterize triangles,
//! lines, and rectangles into a 2D frame buffer. Games render 3D graphics by using the GTE to
//! compute the scene geometry and then using the GPU to rasterize the geometry.
//!
//! Rasterization can use flat shading, Gouraud shading (color interpolation), or texture mapping.
//! Texture mappings can use raw texels (texture pixels) or they can modulate the texel colors.

mod gp0;
mod gp1;
mod registers;
mod render;

use crate::gpu::gp0::{Gp0CommandState, Gp0State};
use crate::gpu::registers::{Registers, VerticalResolution};
use crate::gpu::render::WgpuResources;
use crate::scheduler::Scheduler;
use crate::timers::Timers;
use proc_macros::SaveState;
use std::rc::Rc;

pub use render::DisplayConfig;

const VRAM_LEN_HALFWORDS: usize = 1024 * 512;

type Vram = [u16; VRAM_LEN_HALFWORDS];

#[derive(Debug, SaveState)]
pub struct Gpu {
    vram: Box<Vram>,
    registers: Registers,
    gp0: Gp0State,
    gpu_read_buffer: u32,
    #[save_state(skip)]
    wgpu_resources: WgpuResources,
}

impl Gpu {
    pub fn new(
        wgpu_device: Rc<wgpu::Device>,
        wgpu_queue: Rc<wgpu::Queue>,
        display_config: DisplayConfig,
    ) -> Self {
        let wgpu_resources = WgpuResources::new(wgpu_device, wgpu_queue, display_config);

        Self {
            vram: vec![0; VRAM_LEN_HALFWORDS].into_boxed_slice().try_into().unwrap(),
            registers: Registers::new(),
            gp0: Gp0State::new(),
            gpu_read_buffer: 0,
            wgpu_resources,
        }
    }

    pub fn read_port(&mut self) -> u32 {
        if let Gp0CommandState::SendingToCpu(fields) = self.gp0.command_state {
            self.gpu_read_buffer = self.read_vram_word_for_cpu(fields);
        }

        self.gpu_read_buffer
    }

    pub fn read_status_register(&self, timers: &mut Timers, scheduler: &mut Scheduler) -> u32 {
        let status = self.registers.read_status(&self.gp0, timers, scheduler);
        log::trace!("GPU status register read: {status:08X}");
        status
    }

    pub fn write_gp0_command(&mut self, value: u32) {
        self.handle_gp0_write(value);
    }

    pub fn write_gp1_command(
        &mut self,
        value: u32,
        timers: &mut Timers,
        scheduler: &mut Scheduler,
    ) {
        self.handle_gp1_write(value, timers, scheduler);
    }

    pub fn generate_frame_texture(&mut self) -> &wgpu::Texture {
        self.write_frame_texture()
    }

    pub fn pixel_aspect_ratio(&self) -> f64 {
        if self.wgpu_resources.display_config.dump_vram {
            return 1.0;
        }

        // Target 64:49 screen aspect ratio after accounting for vertical overscan
        let dot_clock_divider: f64 = self.registers.dot_clock_divider().into();
        let normal_ratio = 64.0 / 49.0 / (2560.0 / dot_clock_divider / 224.0);

        if self.registers.interlaced && self.registers.v_resolution == VerticalResolution::Double {
            2.0 * normal_ratio
        } else {
            normal_ratio
        }
    }

    pub fn update_display_config(&mut self, display_config: DisplayConfig) {
        self.wgpu_resources.display_config = display_config;
    }

    pub fn get_wgpu_resources(&self) -> (Rc<wgpu::Device>, Rc<wgpu::Queue>, DisplayConfig) {
        (
            Rc::clone(&self.wgpu_resources.device),
            Rc::clone(&self.wgpu_resources.queue),
            self.wgpu_resources.display_config,
        )
    }

    pub fn from_state(
        state: GpuState,
        wgpu_device: Rc<wgpu::Device>,
        wgpu_queue: Rc<wgpu::Queue>,
        display_config: DisplayConfig,
    ) -> Self {
        Self {
            vram: state.vram,
            registers: state.registers,
            gp0: state.gp0,
            gpu_read_buffer: state.gpu_read_buffer,
            wgpu_resources: WgpuResources::new(wgpu_device, wgpu_queue, display_config),
        }
    }
}
