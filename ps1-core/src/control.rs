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

    pub fn read_io_register(&mut self, address: u32) -> u32 {
        match address & 0xFFFF {
            0x1074 => self.read_interrupt_mask(),
            _ => panic!("I/O register read {address:08X}"),
        }
    }

    pub fn write_io_register(&mut self, address: u32, value: u32) {
        match address & 0xFFFF {
            0x1000 => self.write_expansion_1_address(value),
            0x1004 => self.write_expansion_2_address(value),
            0x1008 => self.write_expansion_1_memory_control(value),
            0x100C => self.write_expansion_3_memory_control(value),
            0x1010 => self.write_bios_memory_control(value),
            0x1014 => self.write_spu_memory_control(value),
            0x1018 => self.write_cdrom_memory_control(value),
            0x101C => self.write_expansion_2_memory_control(value),
            0x1020 => self.write_common_delay(value),
            0x1060 => self.write_ram_size(value),
            0x1070 => self.write_interrupt_status(value),
            0x1074 => self.write_interrupt_mask(value),
            0x1100..=0x110F => {
                log::warn!("Unhandled dot clock register write {address:08X} {value:08X}")
            }
            0x1110..=0x111F => log::warn!(
                "Unhandled horizontal retrace timer register write {address:08x} {value:08X}"
            ),
            0x1120..=0x112F => {
                log::warn!("Unhandled 1/8 system timer register write {address:08X} {value:08X}")
            }
            _ => panic!("I/O register write {address:08X} {value:08X}"),
        }
    }

    fn write_expansion_1_address(&mut self, value: u32) {
        // Bits 24-31 are fixed to $1F and not writable
        self.expansion_1_base_addr = 0x1F000000 | (value & 0x00FFFFFF);
        log::trace!(
            "Expansion 1 base address write: {:08X}",
            self.expansion_1_base_addr
        );
    }

    fn write_expansion_2_address(&mut self, value: u32) {
        // Writing any value other than $1F802000 apparently disables the Expansion 2 region
        self.expansion_2_enabled = value == 0x1F802000;
        log::trace!("Expansion 2 enabled: {}", self.expansion_2_enabled);
    }

    fn write_expansion_1_memory_control(&mut self, value: u32) {
        log::warn!("Unhandled write to Expansion 1 memory control register: {value:08X}");
    }

    fn write_expansion_2_memory_control(&mut self, value: u32) {
        log::warn!("Unhandled write to Expansion 2 memory control register: {value:08X}");
    }

    fn write_expansion_3_memory_control(&mut self, value: u32) {
        log::warn!("Unhandled write to Expansion 3 memory control register: {value:08X}");
    }

    fn write_bios_memory_control(&mut self, value: u32) {
        log::warn!("Unhandled write to BIOS ROM memory control register: {value:08X}");
    }

    fn write_spu_memory_control(&mut self, value: u32) {
        log::warn!("Unhandled write to SPU memory control register: {value:08X}");
    }

    fn write_cdrom_memory_control(&mut self, value: u32) {
        log::warn!("Unhandled write to CD-ROM memory control register: {value:08X}");
    }

    fn write_common_delay(&mut self, value: u32) {
        log::warn!("Unhandled write to common delay register: {value:08X}");
    }

    fn write_ram_size(&mut self, value: u32) {
        log::warn!("Unhandled write to RAM size register: {value:08X}");
    }

    fn write_interrupt_status(&mut self, value: u32) {
        log::warn!("Unhandled write to interrupt status register: {value:08X}");
    }

    fn read_interrupt_mask(&self) -> u32 {
        log::warn!("Unhandled read from interrupt mask register");
        0
    }

    fn write_interrupt_mask(&mut self, value: u32) {
        log::warn!("Unhandled write to interrupt mask register: {value:08X}");
    }
}
