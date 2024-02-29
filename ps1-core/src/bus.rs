use crate::control::ControlRegisters;
use crate::cpu::bus::{BusInterface, OpSize};
use crate::dma::DmaController;
use crate::gpu::Gpu;
use crate::memory::Memory;

pub struct Bus<'a> {
    pub gpu: &'a mut Gpu,
    pub memory: &'a mut Memory,
    pub dma_controller: &'a mut DmaController,
    pub control_registers: &'a mut ControlRegisters,
}

impl<'a> Bus<'a> {
    fn read_io_register(&mut self, address: u32, size: OpSize) -> u32 {
        match address & 0xFFFF {
            0x1040 => unimplemented_register_read("Joypad TX Data", address, size),
            0x104A => {
                // TODO
                0
            }
            0x1070 => self.control_registers.read_interrupt_status(),
            0x1074 => self.control_registers.read_interrupt_mask(),
            0x10F0 => self.dma_controller.read_control(),
            0x1080..=0x10EF => match (address >> 2) & 3 {
                2 => self.dma_controller.read_channel_control(address),
                _ => todo!("DMA register read {address:08X} {size:?}"),
            },
            0x10F4 => self.dma_controller.read_interrupt(),
            0x1110 => unimplemented_register_read("Timer 1 (horizontal retrace)", address, size),
            0x1810 => self.gpu.read_port(),
            0x1814 => self.gpu.read_status_register(),
            0x1C00..=0x1FFF => {
                // TODO SPU registers
                0
            }
            _ => panic!("I/O register read {address:08X} {size:?}"),
        }
    }

    fn write_io_register(&mut self, address: u32, value: u32, size: OpSize) {
        match address & 0xFFFF {
            0x1000 => self.control_registers.write_expansion_1_address(value),
            0x1004 => self.control_registers.write_expansion_2_address(value),
            0x1008 => {
                unimplemented_register_write("Expansion 1 Memory Control", address, value, size)
            }
            0x100C => {
                unimplemented_register_write("Expansion 3 Memory Control", address, value, size)
            }
            0x1010 => unimplemented_register_write("BIOS Memory Control", address, value, size),
            0x1014 => unimplemented_register_write("SPU Memory Control", address, value, size),
            0x1018 => unimplemented_register_write("CD-ROM Memory Control", address, value, size),
            0x101C => {
                unimplemented_register_write("Expansion 2 Memory Control", address, value, size)
            }
            0x1020 => unimplemented_register_write("Common Delay", address, value, size),
            0x104A => unimplemented_register_write("Joypad Control", address, value, size),
            0x1060 => unimplemented_register_write("RAM Size", address, value, size),
            0x1070 => self.control_registers.write_interrupt_status(value),
            0x1074 => self.control_registers.write_interrupt_mask(value),
            0x1080..=0x10EF => match (address >> 2) & 3 {
                0 => self.dma_controller.write_channel_address(address, value),
                1 => self.dma_controller.write_channel_length(address, value),
                2 => self.dma_controller.write_channel_control(
                    address,
                    value,
                    self.gpu,
                    self.memory,
                    self.control_registers,
                ),
                3 => todo!("Invalid DMA register write: {address:08X} {value:08X} {size:?}"),
                _ => unreachable!("value & 3 is always <= 3"),
            },
            0x10F0 => self.dma_controller.write_control(value),
            0x10F4 => self
                .dma_controller
                .write_interrupt(value, self.control_registers),
            0x1100..=0x112F => unimplemented_register_write(
                &format!("Timer {} Register", (address >> 4) & 3),
                address,
                value,
                size,
            ),
            0x1810 => self.gpu.write_gp0_command(value),
            0x1814 => self.gpu.write_gp1_command(value),
            0x1C00..=0x1FFF => {
                // TODO SPU registers
            }
            _ => panic!("I/O register write {address:08X} {value:08X} {size:?}"),
        }
    }
}

fn unimplemented_register_read(name: &str, address: u32, size: OpSize) -> u32 {
    log::warn!("Unimplemented {name} read: {address:08X} {size:?}");
    0
}

fn unimplemented_register_write(name: &str, address: u32, value: u32, size: OpSize) {
    log::warn!("Unimplemented {name} write: {address:08X} {value:08X} {size:?}");
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
            0x1F801000..=0x1F801FFF => self.read_io_register(address, size),
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
                unimplemented_register_write("Expansion Device 1", address, value, size)
            }
            0x1F800000..=0x1F800FFF => {
                self.memory.write_scratchpad_ram(address, value, size);
            }
            0x1F801000..=0x1F801FFF => {
                self.write_io_register(address, value, size);
            }
            0x1F802041 => log::warn!("Unhandled POST write {value:08X} {size:?}"),
            _ => todo!("write {address:08X} {size:?}"),
        }
    }

    fn hardware_interrupt_pending(&self) -> bool {
        self.control_registers.interrupt_pending()
    }
}
