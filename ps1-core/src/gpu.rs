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
pub mod rasterizer;
mod registers;

use crate::gpu::gp0::{Gp0CommandState, Gp0State};
use crate::gpu::rasterizer::naive::NaiveSoftwareRasterizer;
use crate::gpu::registers::{Registers, VerticalResolution};
use crate::scheduler::Scheduler;
use crate::timers::Timers;
use proc_macros::SaveState;
use std::rc::Rc;

use crate::gpu::rasterizer::{Rasterizer, RasterizerInterface};

use crate::gpu::rasterizer::simd::SimdSoftwareRasterizer;
pub use rasterizer::{RasterizerState, RasterizerType};

const VRAM_LEN_HALFWORDS: usize = 1024 * 512;

type Vram = [u16; VRAM_LEN_HALFWORDS];

#[derive(Debug, Clone, Copy)]
pub struct DisplayConfig {
    pub crop_vertical_overscan: bool,
    pub dump_vram: bool,
    pub rasterizer_type: RasterizerType,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            crop_vertical_overscan: true,
            dump_vram: false,
            rasterizer_type: RasterizerType::default(),
        }
    }
}

#[derive(Debug)]
pub struct WgpuResources {
    pub device: Rc<wgpu::Device>,
    pub queue: Rc<wgpu::Queue>,
    pub display_config: DisplayConfig,
}

#[derive(Debug, SaveState)]
pub struct Gpu {
    registers: Registers,
    gp0: Gp0State,
    gpu_read_buffer: u32,
    #[save_state(skip)]
    wgpu_resources: WgpuResources,
    #[save_state(to = RasterizerState)]
    rasterizer: Rasterizer,
}

#[must_use]
fn check_rasterizer_type(rasterizer_type: RasterizerType) -> RasterizerType {
    if rasterizer_type != RasterizerType::SimdSoftware {
        return rasterizer_type;
    }

    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma")))]
    if rasterizer_type == RasterizerType::SimdSoftware {
        log::error!("Binary was not built with AVX2/FMA support; not using SIMD rasterizer");
        return RasterizerType::NaiveSoftware;
    }

    if !is_x86_feature_detected!("avx2") || !is_x86_feature_detected!("fma") {
        log::error!(
            "Current CPU does not support AVX2 and/or FMA instructions; SIMD rasterizer will not work"
        );
        return RasterizerType::NaiveSoftware;
    }

    rasterizer_type
}

impl Gpu {
    pub fn new(
        wgpu_device: Rc<wgpu::Device>,
        wgpu_queue: Rc<wgpu::Queue>,
        mut display_config: DisplayConfig,
    ) -> Self {
        display_config.rasterizer_type = check_rasterizer_type(display_config.rasterizer_type);

        let rasterizer = match display_config.rasterizer_type {
            RasterizerType::NaiveSoftware => {
                Rasterizer::NaiveSoftware(NaiveSoftwareRasterizer::new(&wgpu_device))
            }
            RasterizerType::SimdSoftware => {
                Rasterizer::SimdSoftware(SimdSoftwareRasterizer::new(&wgpu_device))
            }
        };

        let wgpu_resources =
            WgpuResources { device: wgpu_device, queue: wgpu_queue, display_config };

        Self {
            registers: Registers::new(),
            gp0: Gp0State::new(),
            gpu_read_buffer: 0,
            wgpu_resources,
            rasterizer,
        }
    }

    pub fn read_port(&mut self) -> u32 {
        if let Gp0CommandState::SendingToCpu { buffer_idx, halfwords_remaining } =
            self.gp0.command_state
        {
            self.gpu_read_buffer = self.read_vram_word_for_cpu(buffer_idx, halfwords_remaining);
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
        self.rasterizer.generate_frame_texture(&self.registers, &self.wgpu_resources)
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

    pub fn update_display_config(&mut self, mut display_config: DisplayConfig) {
        display_config.rasterizer_type = check_rasterizer_type(display_config.rasterizer_type);

        let prev_rasterizer_type = self.wgpu_resources.display_config.rasterizer_type;
        self.wgpu_resources.display_config = display_config;

        if prev_rasterizer_type != display_config.rasterizer_type {
            let vram = self.rasterizer.clone_vram();
            self.rasterizer = match display_config.rasterizer_type {
                RasterizerType::NaiveSoftware => Rasterizer::NaiveSoftware(
                    NaiveSoftwareRasterizer::from_vram(&self.wgpu_resources.device, &vram),
                ),
                RasterizerType::SimdSoftware => Rasterizer::SimdSoftware(
                    SimdSoftwareRasterizer::from_vram(&self.wgpu_resources.device, &vram),
                ),
            };
        }
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
        let rasterizer =
            Rasterizer::from_state(state.rasterizer, &wgpu_device, display_config.rasterizer_type);

        Self {
            registers: state.registers,
            gp0: state.gp0,
            gpu_read_buffer: state.gpu_read_buffer,
            wgpu_resources: WgpuResources {
                device: wgpu_device,
                queue: wgpu_queue,
                display_config,
            },
            rasterizer,
        }
    }
}

fn new_vram() -> Box<Vram> {
    vec![0; VRAM_LEN_HALFWORDS].into_boxed_slice().try_into().unwrap()
}
