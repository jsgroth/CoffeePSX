pub mod bus;
mod cp0;
mod instructions;

use crate::cpu::bus::{BusInterface, OpSize};
use cp0::SystemControlCoprocessor;

const RESET_VECTOR: u32 = 0xBFC0_0000;

#[derive(Debug, Clone)]
struct Registers {
    gpr: [u32; 32],
    pc: u32,
    hi: u32,
    lo: u32,
    delayed_branch: Option<u32>,
}

impl Registers {
    fn new() -> Self {
        Self {
            gpr: [0; 32],
            pc: RESET_VECTOR,
            hi: 0,
            lo: 0,
            delayed_branch: None,
        }
    }

    fn write_gpr(&mut self, register: u32, value: u32) {
        if register != 0 {
            self.gpr[register as usize] = value;
        }
    }
}

#[derive(Debug, Clone)]
pub struct R3000 {
    registers: Registers,
    cp0: SystemControlCoprocessor,
}

impl R3000 {
    pub fn new() -> Self {
        Self {
            registers: Registers::new(),
            cp0: SystemControlCoprocessor::new(),
        }
    }

    pub fn execute_instruction<B: BusInterface>(&mut self, bus: &mut B) {
        let pc = self.registers.pc;
        let opcode = self.bus_read(bus, pc, OpSize::Word);
        self.registers.pc = match self.registers.delayed_branch.take() {
            Some(address) => address,
            None => pc.wrapping_add(4),
        };

        self.execute_opcode(opcode, pc, bus);
    }

    fn bus_read<B: BusInterface>(&mut self, bus: &mut B, address: u32, size: OpSize) -> u32 {
        match address {
            // kuseg (only first 512MB are valid addresses)
            0x00000000..=0x1FFFFFFF => bus.read(address, size),
            // kseg0
            0x80000000..=0x9FFFFFFF => bus.read(address & 0x1FFFFFFF, size),
            // kseg1
            0xA0000000..=0xBFFFFFFF => bus.read(address & 0x1FFFFFFF, size),
            // cache control register in kseg2
            0xFFFE0130 => todo!("cache control read {address:08X} {size:?}"),
            // other addresses in kuseg and kseg2 are invalid
            _ => todo!("invalid address read {address:08X} {size:?}"),
        }
    }

    fn bus_write<B: BusInterface>(&mut self, bus: &mut B, address: u32, value: u32, size: OpSize) {
        match address {
            // kuseg (only first 512MB are valid addresses)
            0x00000000..=0x1FFFFFFF => bus.write(address, value, size),
            // kseg0
            0x80000000..=0x9FFFFFFF => bus.write(address & 0x1FFFFFFF, value, size),
            // kseg1
            0xA0000000..=0xBFFFFFFF => bus.write(address & 0x1FFFFFFF, value, size),
            // cache control register in kseg2
            0xFFFE0130 => self.cp0.cache_control.write(value),
            // other addresses in kuseg and kseg2 are invalid
            _ => todo!("invalid address write {address:08X} {value:08X} {size:?}"),
        }
    }
}
