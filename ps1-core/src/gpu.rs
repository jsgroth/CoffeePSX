mod gp0;
mod gp1;
mod registers;

use crate::gpu::gp0::{Gp0CommandState, Gp0State};
use crate::gpu::registers::Registers;

const VRAM_LEN: usize = 1024 * 1024;

type Vram = [u8; VRAM_LEN];

#[derive(Debug, Clone)]
pub struct Gpu {
    vram: Box<Vram>,
    registers: Registers,
    gp0_state: Gp0State,
    gpu_read_buffer: u32,
}

impl Gpu {
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_LEN].into_boxed_slice().try_into().unwrap(),
            registers: Registers::new(),
            gp0_state: Gp0State::new(),
            gpu_read_buffer: 0,
        }
    }

    pub fn read_port(&mut self) -> u32 {
        if let Gp0CommandState::SendingToCpu(fields) = self.gp0_state.command_state {
            self.gpu_read_buffer = self.read_vram_word(fields);
        }

        self.gpu_read_buffer
    }

    pub fn read_status_register(&self) -> u32 {
        self.registers.read_status()
    }

    pub fn vram(&self) -> &[u8] {
        self.vram.as_ref()
    }
}
