mod gp0;
mod gp1;
mod registers;

use crate::control::{ControlRegisters, InterruptType};
use crate::gpu::gp0::{Gp0CommandState, Gp0State};
use crate::gpu::registers::Registers;
use crate::timers::Timers;

const VRAM_LEN: usize = 1024 * 1024;

type Vram = [u8; VRAM_LEN];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickEffect {
    None,
    RenderFrame,
}

#[derive(Debug, Clone, Default)]
struct ClockState {
    line: u16,
    line_cycle: u16,
    cpu_cycles_11x: u32,
    odd_frame: bool,
}

impl ClockState {
    fn tick(
        &mut self,
        cpu_cycles: u32,
        control_registers: &mut ControlRegisters,
        timers: &mut Timers,
    ) -> TickEffect {
        // TODO optimize/clean this
        // GPU clock speed is 11/7 times the CPU clock speed
        let mut tick_effect = TickEffect::None;

        self.cpu_cycles_11x += 11 * cpu_cycles;
        while self.cpu_cycles_11x >= 7 {
            self.cpu_cycles_11x -= 7;

            self.line_cycle += 1;
            if self.line_cycle == 3413 {
                self.line_cycle = 0;
                timers.timer_1.increment();

                self.line += 1;
                if self.line == 263 {
                    self.line = 0;
                    self.odd_frame = !self.odd_frame;
                    tick_effect = TickEffect::RenderFrame;
                } else if self.line == 256 {
                    control_registers.set_interrupt_flag(InterruptType::VBlank);
                }
            }
        }

        tick_effect
    }
}

#[derive(Debug, Clone)]
pub struct Gpu {
    vram: Box<Vram>,
    registers: Registers,
    gp0: Gp0State,
    gpu_read_buffer: u32,
    clock_state: ClockState,
}

impl Gpu {
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_LEN].into_boxed_slice().try_into().unwrap(),
            registers: Registers::new(),
            gp0: Gp0State::new(),
            gpu_read_buffer: 0,
            clock_state: ClockState::default(),
        }
    }

    pub fn read_port(&mut self) -> u32 {
        if let Gp0CommandState::SendingToCpu(fields) = self.gp0.command_state {
            self.gpu_read_buffer = self.read_vram_word_for_cpu(fields);
        }

        self.gpu_read_buffer
    }

    pub fn read_status_register(&self) -> u32 {
        let status = self.registers.read_status(&self.gp0, &self.clock_state);
        log::trace!("GPU status register read: {status:08X}");
        status
    }

    pub fn vram(&self) -> &[u8] {
        self.vram.as_ref()
    }

    pub fn tick(
        &mut self,
        cpu_cycles: u32,
        control_registers: &mut ControlRegisters,
        timers: &mut Timers,
    ) -> TickEffect {
        // TODO do actual rendering in here when VBlank is reached
        self.clock_state.tick(cpu_cycles, control_registers, timers)
    }
}
