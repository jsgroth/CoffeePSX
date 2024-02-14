pub mod bus;
mod cp0;
mod instructions;

use crate::bus::{BusInterface, OpSize};
use crate::cp0::SystemControlCoprocessor;

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

    pub fn set_pc(&mut self, pc: u32) {
        self.registers.pc = pc;
    }

    pub fn execute_instruction<B: BusInterface>(&mut self, bus: &mut B) {
        let opcode = bus.read(self.registers.pc, OpSize::Word);
        self.registers.pc = match self.registers.delayed_branch.take() {
            Some(address) => address,
            None => self.registers.pc.wrapping_add(4),
        };

        self.execute_opcode(opcode, bus);
    }
}
