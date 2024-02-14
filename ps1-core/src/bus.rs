use crate::cpu::bus::{BusInterface, OpSize};
use crate::memory::Memory;

pub struct Bus<'a> {
    pub memory: &'a mut Memory,
}

impl<'a> BusInterface for Bus<'a> {
    // TODO memory control for main RAM and BIOS ROM
    // TODO I-cache for opcode reads
    fn read(&mut self, address: u32, size: OpSize) -> u32 {
        match address {
            0x00000000..=0x007FFFFF | 0x80000000..=0x807FFFFF | 0xB0000000..=0xB07FFFFF => {
                self.memory.read_main_ram(address, size)
            }
            0x1FC00000..=0x1FFFFFFF | 0x9FC00000..=0x9FFFFFFF | 0xBFC00000..=0xBFFFFFFF => {
                self.memory.read_bios_rom(address, size)
            }
            _ => todo!("read {address:08X} {size:?}"),
        }
    }

    fn write(&mut self, address: u32, value: u32, size: OpSize) {
        match address {
            0x00000000..=0x007FFFFF | 0x80000000..=0x807FFFFF | 0xB0000000..=0xB07FFFFF => {
                self.memory.write_main_ram(address, value, size);
            }
            _ => todo!("write {address:08X} {size:?}"),
        }
    }
}
