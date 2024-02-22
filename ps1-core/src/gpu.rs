mod gp0;
mod gp1;
mod registers;

use crate::gpu::registers::Registers;

const VRAM_LEN: usize = 1024 * 1024;

type Vram = [u8; VRAM_LEN];

#[derive(Debug, Clone)]
pub struct Gpu {
    vram: Box<Vram>,
    registers: Registers,
}

impl Gpu {
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_LEN].into_boxed_slice().try_into().unwrap(),
            registers: Registers::new(),
        }
    }

    pub fn read_status_register(&self) -> u32 {
        self.registers.read_status()
    }

    pub fn write_gp0_command(&mut self, value: u32) {
        // Highest 3 bits of word determine command, except for some miscellaneous commands
        match value >> 29 {
            5 => todo!("GP0 CPU-to-VRAM blit {value:08X}"),
            _ => todo!("GP0 command {value:08X}"),
        }
    }

    pub fn write_gp1_command(&mut self, value: u32) {
        // Highest 8 bits of word determine command
        match value >> 24 {
            0x00 => self.reset(),
            0x01 => self.clear_command_buffer(),
            0x02 => self.acknowledge_interrupt(),
            0x03 => self.set_display_enabled(value),
            0x04 => self.set_dma_mode(value),
            0x05 => self.set_display_area_start(value),
            0x06 => self.set_horizontal_display_range(value),
            0x07 => self.set_vertical_display_range(value),
            0x08 => self.set_display_mode(value),
            _ => todo!("GP1 command {value:08X}"),
        }
    }
}
