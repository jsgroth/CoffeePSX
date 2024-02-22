use crate::bus::Bus;
use crate::control::ControlRegisters;
use crate::cpu::R3000;
use crate::dma::DmaController;
use crate::memory::Memory;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Ps1Error {
    #[error("Incorrect BIOS ROM size; expected 512KB, was {bios_len}")]
    IncorrectBiosSize { bios_len: usize },
}

pub type Ps1Result<T> = Result<T, Ps1Error>;

#[derive(Debug)]
pub struct Ps1Emulator {
    cpu: R3000,
    memory: Memory,
    dma_controller: DmaController,
    control_registers: ControlRegisters,
}

impl Ps1Emulator {
    pub fn new(bios_rom: Vec<u8>) -> Ps1Result<Self> {
        let memory = Memory::new(bios_rom)?;

        Ok(Self {
            cpu: R3000::new(),
            memory,
            dma_controller: DmaController::new(),
            control_registers: ControlRegisters::new(),
        })
    }

    pub fn tick(&mut self) {
        self.cpu.execute_instruction(&mut Bus {
            memory: &mut self.memory,
            dma_controller: &mut self.dma_controller,
            control_registers: &mut self.control_registers,
        });
    }
}
