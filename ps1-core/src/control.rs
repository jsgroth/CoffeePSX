#[derive(Debug, Clone)]
pub struct ControlRegisters {
    expansion_1_base_addr: u32,
    expansion_2_enabled: bool,
}

impl ControlRegisters {
    pub fn new() -> Self {
        Self {
            expansion_1_base_addr: 0x1F000000,
            expansion_2_enabled: true,
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
}
