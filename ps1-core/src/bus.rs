use crate::control::ControlRegisters;
use crate::cpu::bus::{BusInterface, OpSize};
use crate::memory::Memory;

pub struct Bus<'a> {
    pub memory: &'a mut Memory,
    pub control_registers: &'a mut ControlRegisters,
}

impl<'a> BusInterface for Bus<'a> {
    // TODO memory control for main RAM and BIOS ROM
    // TODO I-cache for opcode reads
    fn read(&mut self, address: u32, size: OpSize) -> u32 {
        match address {
            0x00000000..=0x007FFFFF => self.memory.read_main_ram(address, size),
            0x1F000000..=0x1F7FFFFF => {
                log::warn!("Unhandled expansion 1 read {address:08X} {size:?}");
                0
            }
            0x1F800000..=0x1F800FFF => self.memory.read_scratchpad_ram(address, size),
            0x1F801000..=0x1F8017FF => self.control_registers.read_io_register(address),
            0x1FC00000..=0x1FFFFFFF => self.memory.read_bios_rom(address, size),
            _ => todo!("read {address:08X} {size:?}"),
        }
    }

    fn write(&mut self, address: u32, value: u32, size: OpSize) {
        match address {
            0x00000000..=0x007FFFFF => {
                self.memory.write_main_ram(address, value, size);
            }
            0x1F000000..=0x1F7FFFFF => {
                log::warn!("Unhandled expansion 1 write {address:08X} {value:08X} {size:?}")
            }
            0x1F800000..=0x1F800FFF => {
                self.memory.write_scratchpad_ram(address, value, size);
            }
            0x1F801000..=0x1F8017FF => {
                self.control_registers.write_io_register(address, value);
            }
            0x1F801D80..=0x1F801FFF => {
                log::warn!("Unhandled SPU register write {address:08X} {value:08X} {size:?}")
            }
            0x1F802041 => log::warn!("Unhandled POST write {value:08X} {size:?}"),
            _ => todo!("write {address:08X} {size:?}"),
        }
    }
}
