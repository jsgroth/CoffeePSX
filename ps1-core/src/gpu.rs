//! PS1 GPU (Graphics Processing Unit)

mod gp0;
mod gp1;
mod registers;

use crate::api::RenderParams;
use crate::gpu::gp0::{Gp0CommandState, Gp0State};
use crate::gpu::registers::{Registers, VerticalResolution};
use crate::scheduler::Scheduler;
use crate::timers::Timers;
use bincode::{Decode, Encode};

const VRAM_LEN: usize = 1024 * 1024;

type Vram = [u8; VRAM_LEN];

#[derive(Debug, Clone, Encode, Decode)]
pub struct Gpu {
    vram: Box<Vram>,
    registers: Registers,
    gp0: Gp0State,
    gpu_read_buffer: u32,
}

impl Gpu {
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_LEN].into_boxed_slice().try_into().unwrap(),
            registers: Registers::new(),
            gp0: Gp0State::new(),
            gpu_read_buffer: 0,
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

    pub fn vram(&self) -> &[u8] {
        self.vram.as_ref()
    }

    pub fn render_params(&self) -> RenderParams {
        let (x1, x2) = self.registers.x_display_range;
        let (y1, y2) = self.registers.y_display_range;

        RenderParams {
            frame_x: self.registers.display_area_x,
            frame_y: self.registers.display_area_y,
            frame_width: if self.registers.force_h_368px {
                368
            } else {
                self.registers.h_resolution.to_pixels().into()
            },
            frame_height: if self.registers.interlaced
                && self.registers.v_resolution == VerticalResolution::Double
            {
                480
            } else {
                240
            },
            display_x_offset: x1 as i32 - 0x260,
            display_y_offset: y1 as i32 - 16,
            display_width: if x2 < x1 {
                0
            } else {
                (x2 - x1) / u32::from(self.registers.dot_clock_divider())
            },
            display_height: if y2 < y1 { 0 } else { y2 - y1 },
            display_enabled: self.registers.display_enabled,
        }
    }
}
