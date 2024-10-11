//! LSI CW33300, the PS1 CPU
//!
//! Uses the MIPS I instruction set and is binary-compatible with the R3000. Includes 2 MIPS
//! coprocessors, the standard System Control Processor (CP0) and a 3D math coprocessor called the
//! Geometry Transformation Engine (CP2, or usually GTE).

mod cp0;
mod gte;
mod icache;
mod instructions;

use crate::bus::Bus;
use crate::cpu::cp0::ExceptionCode;
use crate::cpu::gte::GeometryTransformationEngine;
use crate::cpu::icache::InstructionCache;
use crate::num::U32Ext;
use crate::pgxp::{PgxpConfig, PgxpCpuRegisters};
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
    next_pc: u32,
    in_delay_slot: bool,
    hi: u32,
    lo: u32,
    delayed_load: (u32, u32),
    delayed_load_next: (u32, u32),
}

impl Registers {
    fn new() -> Self {
        Self {
            gpr: [0; 32],
            pc: RESET_VECTOR,
            next_pc: RESET_VECTOR + 4,
            in_delay_slot: false,
            hi: 0,
            lo: 0,
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
        if self.delayed_load.0 != 0 {
            let (register, value) = self.delayed_load;
            self.gpr[register as usize] = value;
            self.delayed_load.0 = 0;
        }

        if self.delayed_load_next.0 != 0 {
            self.delayed_load = mem::take(&mut self.delayed_load_next);
        }
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

impl OpSize {
    pub fn mask(self, value: u32) -> u32 {
        match self {
            Self::Byte => value & 0xFF,
            Self::HalfWord => value & 0xFFFF,
            Self::Word => value,
        }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct R3000 {
    registers: Registers,
    pgxp: PgxpCpuRegisters,
    pgxp_config: PgxpConfig,
    i_cache: Box<InstructionCache>,
    cp0: SystemControlCoprocessor,
    gte: GeometryTransformationEngine,
    instruction_cycles: u32,
}

const BIU_CACHE_CONTROL_ADDR: u32 = 0xFFFE0130;

macro_rules! impl_bus_write {
    ($name:ident, $write_fn:ident, $memory_cycles_fn:ident) => {
        fn $name(&mut self, bus: &mut Bus<'_>, address: u32, value: u32) {
            if self.cp0.status.isolate_cache {
                // If cache is isolated, send writes directly to instruction cache
                // The BIOS isolates cache as part of the flushCache() kernel function
                if self.cp0.cache_control.tag_test_mode {
                    self.i_cache.invalidate_tag(address);
                } else {
                    self.i_cache.write_opcode(address, value);
                }
                return;
            }

            if address == BIU_CACHE_CONTROL_ADDR {
                self.cp0.cache_control.write(value);
                return;
            }

            if address.bit(29) {
                // Write to uncached address (kseg1)
                self.instruction_cycles += $memory_cycles_fn(address);
            }

            validate_address(address);
            bus.$write_fn(address & 0x1FFFFFFF, value);
        }
    };
}

impl R3000 {
    pub fn new(pgxp_config: PgxpConfig) -> Self {
        Self {
            registers: Registers::new(),
            pgxp: PgxpCpuRegisters::new(),
            pgxp_config,
            i_cache: Box::new(InstructionCache::new()),
            cp0: SystemControlCoprocessor::new(),
            gte: GeometryTransformationEngine::new(pgxp_config),
            instruction_cycles: 0,
        }
    }

    pub fn update_pgxp_config(&mut self, pgxp_config: PgxpConfig) {
        self.pgxp_config = pgxp_config;
        self.gte.update_pgxp_config(pgxp_config);
    }

    pub fn pc(&self) -> u32 {
        self.registers.pc
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.registers.pc = pc;
        self.registers.next_pc = pc.wrapping_add(4);
        self.registers.in_delay_slot = false;
    }

    pub fn get_gpr(&self, register: u32) -> u32 {
        self.registers.gpr[register as usize]
    }

    pub fn set_gpr(&mut self, register: u32, value: u32) {
        self.registers.write_gpr(register, value);
    }

    #[must_use]
    pub fn execute_instruction(&mut self, bus: &mut Bus<'_>) -> u32 {
        self.instruction_cycles = 1;

        let pc = self.registers.pc;
        let in_delay_slot = self.registers.in_delay_slot;

        if pc & 3 != 0 {
            // Address error on opcode fetch
            self.handle_exception(Exception::AddressErrorLoad(pc), pc, in_delay_slot);
            self.process_delayed_loads();

            // TODO pure guess at exception timing
            return 3;
        }

        // Opcode is always read, even if an exception will be handled
        let opcode = self.fetch_opcode(bus, pc);

        self.cp0.cause.set_hardware_interrupt_flag(bus.hardware_interrupt_pending());
        if self.cp0.interrupt_pending() {
            // If the PC currently points to a GTE opcode, it needs to be executed before handling
            // the exception because the exception handler will typically skip over it when returning.
            // Some games depend on this for correct geometry, e.g. Crash Bandicoot and Final Fantasy 7
            if is_gte_command_opcode(opcode) {
                let _ = self.execute_opcode(opcode, pc, bus);
            }

            self.handle_exception(Exception::Interrupt, pc, in_delay_slot);
            self.process_delayed_loads();
            return self.instruction_cycles;
        }

        self.registers.pc = self.registers.next_pc;
        self.registers.next_pc = self.registers.pc.wrapping_add(4);
        self.registers.in_delay_slot = false;

        if let Err(exception) = self.execute_opcode(opcode, pc, bus) {
            self.handle_exception(exception, pc, in_delay_slot);
        }

        self.process_delayed_loads();

        self.instruction_cycles
    }

    fn process_delayed_loads(&mut self) {
        self.registers.process_delayed_loads();

        if self.pgxp_config.enabled {
            self.pgxp.process_delayed_loads();
        }
    }

    fn fetch_opcode(&mut self, bus: &mut Bus<'_>, address: u32) -> u32 {
        validate_address(address);

        // kuseg ($00000000-$1FFFFFFF) and kseg0 ($80000000-$9FFFFFFF) are cacheable
        // kseg1 ($A0000000-$BFFFFFFF) is not cacheable
        if address.bit(29) {
            self.instruction_cycles += memory_access_cycles_u32(address);
            return bus.read_u32(address & 0x1FFFFFFF);
        }

        // I-cache is based on physical address, which for PS1 means just drop the highest 3 bits
        let address = address & 0x1FFFFFFF;

        if let Some(opcode) = self.i_cache.check_cache(address) {
            return opcode;
        }

        // The hardware seems to be able to read cache lines much faster than it would take to read
        // the 4 individual words; not accounting for this will cause slowdown in some games
        self.instruction_cycles += 3 + memory_access_cycles_u32(address);

        // If opcode not found in I-cache, fetch the full current cache line
        self.i_cache.update_tag(address);

        let mut cache_addr = address & !0xF;
        for _ in 0..4 {
            let opcode = bus.read_u32(cache_addr);
            self.i_cache.write_opcode(cache_addr, opcode);
            cache_addr += 4;
        }

        self.i_cache.get_opcode_no_tag_check(address)
    }

    fn bus_read_u8(&mut self, bus: &mut Bus<'_>, address: u32) -> u32 {
        self.instruction_cycles += memory_access_cycles_u8(address);

        validate_address(address);
        bus.read_u8(address & 0x1FFFFFFF)
    }

    fn bus_read_u16(&mut self, bus: &mut Bus<'_>, address: u32) -> u32 {
        self.instruction_cycles += memory_access_cycles_u16(address);

        validate_address(address);
        bus.read_u16(address & 0x1FFFFFFF)
    }

    fn bus_read_u32(&mut self, bus: &mut Bus<'_>, address: u32) -> u32 {
        self.instruction_cycles += memory_access_cycles_u32(address);

        validate_address(address);
        bus.read_u32(address & 0x1FFFFFFF)
    }

    impl_bus_write!(bus_write_u8, write_u8, memory_access_cycles_u8);
    impl_bus_write!(bus_write_u16, write_u16, memory_access_cycles_u16);
    impl_bus_write!(bus_write_u32, write_u32, memory_access_cycles_u32);

    fn handle_exception(&mut self, exception: Exception, pc: u32, in_delay_slot: bool) {
        self.cp0.handle_exception(exception, pc, in_delay_slot);

        self.registers.pc = if self.cp0.status.boot_exception_vectors {
            BOOT_EXCEPTION_VECTOR
        } else {
            EXCEPTION_VECTOR
        };
        self.registers.next_pc = self.registers.pc.wrapping_add(4);
        self.registers.in_delay_slot = false;
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

macro_rules! impl_memory_access_cycles {
    ($name:ident, $bios_cycles:expr) => {
        fn $name(address: u32) -> u32 {
            match address & 0x1FFFFFFF {
                // Main RAM
                0x00000000..=0x007FFFFF => 5,
                // Scratchpad RAM
                0x1F800000..=0x1F8003FF => 0,
                // BIOS ROM
                0x1FC00000..=0x1FFFFFFF => $bios_cycles,
                // I/O registers
                _ => 3,
            }
        }
    };
}

impl_memory_access_cycles!(memory_access_cycles_u8, 7);
impl_memory_access_cycles!(memory_access_cycles_u16, 13);
impl_memory_access_cycles!(memory_access_cycles_u32, 25);
