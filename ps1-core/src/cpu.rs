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
    delayed_load: Option<(u32, u32)>,
    delayed_load_next: Option<(u32, u32)>,
}

impl Registers {
    fn new() -> Self {
        Self {
            gpr: [0; 32],
            pc: RESET_VECTOR,
            hi: 0,
            lo: 0,
            delayed_branch: None,
            delayed_load: None,
            delayed_load_next: None,
        }
    }

    fn read_gpr_lwl_lwr(&self, register: u32) -> u32 {
        // LWL and LWR are not affected by load delays; they can read in-flight values from load
        // instructions
        match self.delayed_load {
            Some((delayed_register, value)) if register == delayed_register => value,
            _ => self.gpr[register as usize],
        }
    }

    fn write_gpr(&mut self, register: u32, value: u32) {
        if register != 0 {
            self.gpr[register as usize] = value;
        }
    }

    fn write_gpr_delayed(&mut self, register: u32, value: u32) {
        if register != 0 {
            // Undocumented: If two consecutive load instructions write to the same register, the
            // first delayed load is canceled
            if self
                .delayed_load
                .is_some_and(|(delayed_register, _)| register == delayed_register)
            {
                self.delayed_load = None;
            }
            self.delayed_load_next = Some((register, value));
        }
    }

    fn process_delayed_loads(&mut self) {
        if let Some((register, value)) = self.delayed_load {
            self.gpr[register as usize] = value;
        }
        self.delayed_load = self.delayed_load_next.take();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exception {
    AddressErrorLoad(u32),
    AddressErrorStore(u32),
    Syscall,
    Breakpoint,
    ArithmeticOverflow,
}

impl Exception {
    fn to_code(self) -> ExceptionCode {
        match self {
            Self::AddressErrorLoad(_) => ExceptionCode::AddressErrorLoad,
            Self::AddressErrorStore(_) => ExceptionCode::AddressErrorStore,
            Self::Syscall => ExceptionCode::Syscall,
            Self::Breakpoint => ExceptionCode::Breakpoint,
            Self::ArithmeticOverflow => ExceptionCode::ArithmeticOverflow,
        }
    }
}

type CpuResult<T> = Result<T, Exception>;

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
        self.registers.gpr[register as usize] = value;
    }

    pub fn execute_instruction<B: BusInterface>(&mut self, bus: &mut B) {
        let pc = self.registers.pc;
        if pc & 3 != 0 {
            // Address error on opcode fetch
            self.handle_exception(
                Exception::AddressErrorLoad(pc),
                pc,
                self.registers.delayed_branch.is_some(),
            );
            return;
        }

        let opcode = self.bus_read(bus, pc, OpSize::Word);
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
        log::trace!(
            "Handling exception {exception:?}; PC={pc:08X}, BD={}",
            u8::from(in_delay_slot)
        );

        self.cp0.handle_exception(exception, pc, in_delay_slot);

        self.registers.pc = if self.cp0.status.boot_exception_vectors {
            BOOT_EXCEPTION_VECTOR
        } else {
            EXCEPTION_VECTOR
        };
        self.registers.delayed_branch = None;
    }
}
