//! PS1 control registers (e.g. interrupt registers)

use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptType {
    VBlank,
    CdRom,
    Dma,
    Timer0,
    Timer1,
    Timer2,
    Sio0,
}

impl InterruptType {
    const fn bit_mask(self) -> u16 {
        match self {
            Self::VBlank => 1,
            Self::CdRom => 1 << 2,
            Self::Dma => 1 << 3,
            Self::Timer0 => 1 << 4,
            Self::Timer1 => 1 << 5,
            Self::Timer2 => 1 << 6,
            Self::Sio0 => 1 << 7,
        }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct InterruptRegisters {
    interrupt_mask: u16,
    interrupt_status: u16,
}

impl InterruptRegisters {
    pub fn new() -> Self {
        Self { interrupt_mask: 0, interrupt_status: 0 }
    }

    pub fn read_interrupt_status(&self) -> u32 {
        self.interrupt_status.into()
    }

    pub fn write_interrupt_status(&mut self, value: u32) {
        // Writing 0 to a bit clears it, writing 1 leaves it unchanged
        self.interrupt_status &= value as u16;

        log::debug!("Interrupt status write: {value:04X}");
    }

    pub fn read_interrupt_mask(&self) -> u32 {
        self.interrupt_mask.into()
    }

    pub fn write_interrupt_mask(&mut self, value: u32) {
        self.interrupt_mask = value as u16;

        log::debug!("Interrupt mask register write: {:04X}", self.interrupt_mask);
    }

    pub fn set_interrupt_flag(&mut self, interrupt: InterruptType) {
        self.interrupt_status |= interrupt.bit_mask();

        log::debug!("Set interrupt status flag: {interrupt:?}");
    }

    pub fn interrupt_pending(&self) -> bool {
        self.interrupt_mask & self.interrupt_status != 0
    }
}
