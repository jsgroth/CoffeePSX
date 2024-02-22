pub mod bus;
mod cp0;
mod instructions;

use crate::cpu::bus::{BusInterface, OpSize};
use crate::cpu::cp0::ExceptionCode;
use cp0::SystemControlCoprocessor;

const RESET_VECTOR: u32 = 0xBFC0_0000;
const EXCEPTION_VECTOR: u32 = 0x8000_0080;
const BOOT_EXCEPTION_VECTOR: u32 = 0xBFC0_0180;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exception {
    Syscall,
}

impl Exception {
    fn to_code(self) -> ExceptionCode {
        match self {
            Self::Syscall => ExceptionCode::Syscall,
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
        let (in_delay_slot, next_pc) = match self.registers.delayed_branch.take() {
            Some(address) => (true, address),
            None => (false, pc.wrapping_add(4)),
        };
        self.registers.pc = next_pc;

        if let Err(exception) = self.execute_opcode(opcode, pc, bus) {
            self.handle_exception(exception, pc, in_delay_slot);
        }
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
        log::trace!("Bus write {address:08X} {value:08X} {size:?}");

        if self.cp0.status.isolate_cache {
            // If cache is isolated, send writes directly to scratchpad RAM
            // The BIOS isolates cache on startup to zero out scratchpad
            bus.write(0x1F800000 | (address & 0x3FF), value, size);
            return;
        }

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

    fn handle_exception(&mut self, exception: Exception, pc: u32, in_delay_slot: bool) {
        self.cp0.handle_exception(exception, pc, in_delay_slot);

        self.registers.pc = if self.cp0.status.boot_exception_vectors {
            BOOT_EXCEPTION_VECTOR
        } else {
            EXCEPTION_VECTOR
        };
        self.registers.delayed_branch = None;
    }
}
