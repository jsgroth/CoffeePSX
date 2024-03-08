#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptType {
    VBlank,
    CdRom,
    Dma,
}

impl InterruptType {
    const fn bit_mask(self) -> u16 {
        match self {
            Self::VBlank => 1,
            Self::CdRom => 1 << 2,
            Self::Dma => 1 << 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ControlRegisters {
    expansion_1_base_addr: u32,
    expansion_2_enabled: bool,
    interrupt_mask: u16,
    interrupt_status: u16,
}

impl ControlRegisters {
    pub fn new() -> Self {
        Self {
            expansion_1_base_addr: 0x1F000000,
            expansion_2_enabled: true,
            interrupt_mask: 0,
            interrupt_status: 0,
        }
    }

    pub fn write_expansion_1_address(&mut self, value: u32) {
        // Bits 24-31 are fixed to $1F and not writable
        self.expansion_1_base_addr = 0x1F000000 | (value & 0x00FFFFFF);
        log::trace!(
            "Expansion 1 base address write: {:08X}",
            self.expansion_1_base_addr
        );
    }

    pub fn write_expansion_2_address(&mut self, value: u32) {
        // Writing any value other than $1F802000 apparently disables the Expansion 2 region
        self.expansion_2_enabled = value == 0x1F802000;
        log::trace!("Expansion 2 enabled: {}", self.expansion_2_enabled);
    }

    pub fn read_interrupt_status(&self) -> u32 {
        self.interrupt_status.into()
    }

    pub fn write_interrupt_status(&mut self, value: u32) {
        // Writing 0 to a bit clears it, writing 1 leaves it unchanged
        self.interrupt_status &= value as u16;
    }

    pub fn read_interrupt_mask(&self) -> u32 {
        self.interrupt_mask.into()
    }

    pub fn write_interrupt_mask(&mut self, value: u32) {
        self.interrupt_mask = value as u16;

        log::trace!("Interrupt mask register write: {:04X}", self.interrupt_mask);
    }

    pub fn set_interrupt_flag(&mut self, interrupt: InterruptType) {
        self.interrupt_status |= interrupt.bit_mask();

        log::trace!("Set interrupt status flag: {interrupt:?}");
    }

    pub fn interrupt_pending(&self) -> bool {
        self.interrupt_mask & self.interrupt_status != 0
    }
}
