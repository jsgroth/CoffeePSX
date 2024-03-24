//! LSI CW33300, the PS1 CPU
//!
//! Uses the MIPS I instruction set and is binary-compatible with the R3000

mod cp0;
mod gte;
mod instructions;

use crate::bus::Bus;
use crate::cpu::cp0::ExceptionCode;
use crate::cpu::gte::GeometryTransformationEngine;
use bincode::{Decode, Encode};
use cp0::SystemControlCoprocessor;
use std::mem;

const RESET_VECTOR: u32 = 0xBFC0_0000;
const EXCEPTION_VECTOR: u32 = 0x8000_0080;
const BOOT_EXCEPTION_VECTOR: u32 = 0xBFC0_0180;

#[derive(Debug, Clone, Encode, Decode)]
struct Registers {
    gpr: [u32; 32],
    pc: u32,
    hi: u32,
    lo: u32,
    delayed_branch: Option<u32>,
    delayed_load: (u32, u32),
    delayed_load_next: (u32, u32),
}

impl Registers {
    fn new() -> Self {
        Self {
            gpr: [0; 32],
            pc: RESET_VECTOR,
            hi: 0,
            lo: 0,
            delayed_branch: None,
            delayed_load: (0, 0),
            delayed_load_next: (0, 0),
        }
    }

    fn read_gpr_lwl_lwr(&self, register: u32) -> u32 {
        // LWL and LWR are not affected by load delays; they can read in-flight values from load
        // instructions
        let (delayed_register, delayed_value) = self.delayed_load;
        if delayed_register == register { delayed_value } else { self.gpr[register as usize] }
    }

    fn write_gpr(&mut self, register: u32, value: u32) {
        if register == 0 {
            return;
        }

        self.gpr[register as usize] = value;

        // A non-load register write should discard any in-progress delayed load to that
        // register. Not doing this causes the BIOS to boot incorrectly
        if self.delayed_load.0 == register {
            self.delayed_load = (0, 0);
        }
    }

    fn write_gpr_delayed(&mut self, register: u32, value: u32) {
        if register == 0 {
            return;
        }

        // Undocumented: If two consecutive load instructions write to the same register, the
        // first delayed load is canceled
        if self.delayed_load.0 == register {
            self.delayed_load = (0, 0);
        }
        self.delayed_load_next = (register, value);
    }

    fn process_delayed_loads(&mut self) {
        // No need for an if check here; if register is 0 then value will be 0
        let (register, value) = self.delayed_load;
        self.gpr[register as usize] = value;

        debug_assert!(!(register == 0 && value != 0));

        self.delayed_load = mem::take(&mut self.delayed_load_next);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exception {
    Interrupt,
    AddressErrorLoad(u32),
    AddressErrorStore(u32),
    Syscall,
    Breakpoint,
    ArithmeticOverflow,
}

impl Exception {
    fn to_code(self) -> ExceptionCode {
        match self {
            Self::Interrupt => ExceptionCode::Interrupt,
            Self::AddressErrorLoad(_) => ExceptionCode::AddressErrorLoad,
            Self::AddressErrorStore(_) => ExceptionCode::AddressErrorStore,
            Self::Syscall => ExceptionCode::Syscall,
            Self::Breakpoint => ExceptionCode::Breakpoint,
            Self::ArithmeticOverflow => ExceptionCode::ArithmeticOverflow,
        }
    }
}

type CpuResult<T> = Result<T, Exception>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpSize {
    Byte,
    HalfWord,
    Word,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct R3000 {
    registers: Registers,
    cp0: SystemControlCoprocessor,
    gte: GeometryTransformationEngine,
}

macro_rules! impl_bus_write {
    ($name:ident, $write_fn:ident) => {
        fn $name(&mut self, bus: &mut Bus<'_>, address: u32, value: u32) {
            if self.cp0.status.isolate_cache {
                // If cache is isolated, send writes directly to scratchpad RAM
                // The BIOS isolates cache on startup to zero out scratchpad
                bus.$write_fn(0x1F800000 | (address & 0x3FF), value);
                return;
            }

            if address == 0xFFFE0130 {
                self.cp0.cache_control.write(value);
                return;
            }

            validate_address(address);
            bus.$write_fn(address & 0x1FFFFFFF, value);
        }
    };
}

impl R3000 {
    pub fn new() -> Self {
        Self {
            registers: Registers::new(),
            cp0: SystemControlCoprocessor::new(),
            gte: GeometryTransformationEngine::new(),
        }
    }

    pub fn pc(&self) -> u32 {
        self.registers.pc
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.registers.pc = pc;
        self.registers.delayed_branch = None;
    }

    pub fn get_gpr(&self, register: u32) -> u32 {
        self.registers.gpr[register as usize]
    }

    pub fn set_gpr(&mut self, register: u32, value: u32) {
        self.registers.write_gpr(register, value);
    }

    pub fn execute_instruction(&mut self, bus: &mut Bus<'_>) {
        let pc = self.registers.pc;

        if pc & 3 != 0 {
            // Address error on opcode fetch
            self.handle_exception(
                Exception::AddressErrorLoad(pc),
                pc,
                self.registers.delayed_branch.is_some(),
            );
            self.registers.process_delayed_loads();
            return;
        }

        self.cp0.cause.set_hardware_interrupt_flag(bus.hardware_interrupt_pending());
        if self.cp0.interrupt_pending() {
            // If the PC currently points to a GTE opcode, it needs to be executed before handling
            // the exception because the exception handler will typically skip over it when returning.
            // Some games depend on this for correct geometry, e.g. Crash Bandicoot and Final Fantasy 7
            let opcode = self.bus_read_u32(bus, pc);
            if is_gte_command_opcode(opcode) {
                let _ = self.execute_opcode(opcode, pc, bus);
            }

            self.handle_exception(
                Exception::Interrupt,
                pc,
                self.registers.delayed_branch.is_some(),
            );
            self.registers.process_delayed_loads();
            return;
        }

        let opcode = self.bus_read_u32(bus, pc);
        let (in_delay_slot, next_pc) = match self.registers.delayed_branch.take() {
            Some(address) => (true, address),
            None => (false, pc.wrapping_add(4)),
        };
        self.registers.pc = next_pc;

        if let Err(exception) = self.execute_opcode(opcode, pc, bus) {
            self.handle_exception(exception, pc, in_delay_slot);
        }

        self.registers.process_delayed_loads();
    }

    // TODO handle kuseg vs. kseg0 vs. kseg1
    #[allow(clippy::unused_self)]
    fn bus_read_u8(&self, bus: &mut Bus<'_>, address: u32) -> u32 {
        validate_address(address);
        bus.read_u8(address & 0x1FFFFFFF)
    }

    #[allow(clippy::unused_self)]
    fn bus_read_u16(&self, bus: &mut Bus<'_>, address: u32) -> u32 {
        validate_address(address);
        bus.read_u16(address & 0x1FFFFFFF)
    }

    #[allow(clippy::unused_self)]
    fn bus_read_u32(&self, bus: &mut Bus<'_>, address: u32) -> u32 {
        validate_address(address);
        bus.read_u32(address & 0x1FFFFFFF)
    }

    impl_bus_write!(bus_write_u8, write_u8);
    impl_bus_write!(bus_write_u16, write_u16);
    impl_bus_write!(bus_write_u32, write_u32);

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

fn is_gte_command_opcode(opcode: u32) -> bool {
    // All COP2 opcodes
    opcode & 0xFE000000 == 0x4A000000
}

fn validate_address(address: u32) {
    if (0x20000000..0x80000000).contains(&address) || address >= 0xC0000000 {
        todo!("unimplemented bus address {address:08X}");
    }
}
